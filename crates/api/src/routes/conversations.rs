//! Team and group conversations (ptcore::conversations), and broadcasts.
//! Teammates (the head coach and assistants) message each other one to one
//! and start groups with teammates and the clients they can reach: every
//! client for the head coach, an assistant's assigned clients if they have
//! Messaging. Clients read and reply in groups they're in but don't start
//! them; group messages don't use credits. Membership is the access: only
//! members see a conversation, and a client who leaves the coach, or an
//! assistant who leaves the team, leaves its groups (`forget`).
//!
//! A broadcast is one message sent to many clients' private threads, so no
//! client sees who else got it.

use std::collections::HashSet;

use aws_sdk_dynamodb::types::AttributeValue;
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use futures::future::try_join_all;
use futures::stream::{self, StreamExt};
use ptcore::conversations::{direct_id, group_name, ConvKind, ConvMember, ConvMessage, Conversation, MAX_BROADCAST, MAX_MEMBERS, MAX_TEXT};
use ptcore::model::{Attachment, AttachmentKind, Comment, Profile, Role, RosterEntry};
use ptcore::team::{Member, Perm};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope_as, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::s;
use crate::util::{check_size, ext_for, new_id, presign_get, presign_put, s3_object_exists, untag, AUDIO_TYPES, IMAGE_TYPES, MAX_PHOTO_BYTES, MAX_VOICE_BYTES, PENDING_TAG};
use crate::AppState;

/// Messages returned when a conversation is opened, the latest.
const SHOWN: i32 = 200;
const MAX_VOICE_SECONDS: f64 = 125.0;

/// The business the caller's conversations live in.
fn head(ctx: &Ctx) -> ApiResult<&str> {
    match ctx.profile.role {
        Role::Coach => Ok(ctx.user_id()),
        Role::Assistant | Role::Client => Ok(&ctx.profile.coach_id),
        Role::Admin => Err(ApiError::Forbidden("conversations are for coaches and their clients")),
    }
}

fn summary_key(head: &str, id: &str) -> (String, String) {
    (format!("COACH#{head}"), format!("CONV#{id}"))
}

fn media_prefix(head: &str, id: &str) -> String {
    format!("media/{head}/conversations/{id}/")
}

fn me(ctx: &Ctx) -> ConvMember {
    ConvMember { user_id: ctx.user_id().to_string(), name: ctx.display_name(), role: ctx.profile.role }
}

/// The head coach, or the group's starter, manages a group.
fn can_manage(ctx: &Ctx, c: &Conversation) -> bool {
    c.kind == ConvKind::Group && (ctx.is_coach() || c.created_by == ctx.user_id())
}

fn view(ctx: &Ctx, c: &Conversation) -> Value {
    json!({
        "id": c.id,
        "kind": c.kind,
        "title": c.title_for(ctx.user_id()),
        "name": c.name,
        "members": c.members,
        "createdBy": c.created_by,
        "createdAt": c.created_at,
        "lastAt": c.last_at,
        "lastText": c.last_text,
        "lastAuthor": c.last_author,
        "unread": c.unread_for(ctx.user_id()),
        "canManage": can_manage(ctx, c),
    })
}

/// The caller's conversations, newest first.
pub async fn mine(st: &AppState, ctx: &Ctx) -> ApiResult<Vec<Conversation>> {
    let Ok(head) = head(ctx) else { return Ok(vec![]) };
    let mut all: Vec<Conversation> = st.store.query_prefix(&format!("COACH#{head}"), "CONV#").await?;
    all.retain(|c| c.member(ctx.user_id()).is_some());
    all.sort_by_key(|c| std::cmp::Reverse(c.last_at.unwrap_or(c.created_at)));
    Ok(all)
}

/// A conversation the caller is in; anyone else is told it doesn't exist.
async fn load(st: &AppState, ctx: &Ctx, id: &str) -> ApiResult<(String, Conversation)> {
    let head = head(ctx)?.to_string();
    let (pk, sk) = summary_key(&head, id);
    let c: Conversation = st.store.get(&pk, &sk).await?.ok_or(ApiError::NotFound("conversation"))?;
    if c.member(ctx.user_id()).is_none() {
        return Err(ApiError::NotFound("conversation"));
    }
    Ok((head, c))
}

/// Who the caller may add to a conversation: their teammates, and the
/// clients they can message.
async fn reachable(st: &AppState, ctx: &Ctx) -> ApiResult<Vec<ConvMember>> {
    if !matches!(ctx.profile.role, Role::Coach | Role::Assistant) {
        return Err(ApiError::Forbidden("only coaches start conversations"));
    }
    let head = head(ctx)?;
    let pk = format!("COACH#{head}");
    let mut out = Vec::new();
    if ctx.is_assistant() {
        let p: Option<Profile> = st.store.get(&format!("USER#{head}"), "PROFILE").await?;
        out.extend(p.map(|p| ConvMember { user_id: p.user_id, name: p.name, role: Role::Coach }));
    }
    let team: Vec<Member> = st.store.query_prefix(&pk, "ASSISTANT#").await?;
    out.extend(team.into_iter().filter(|m| m.user_id != ctx.user_id()).map(|m| ConvMember { name: m.display(), user_id: m.user_id, role: Role::Assistant }));
    if ctx.require(Perm::Messaging).is_ok() {
        let roster: Vec<RosterEntry> = st.store.query_prefix(&pk, "CLIENT#").await?;
        out.extend(roster.into_iter().filter(|e| ctx.sees_client(&e.client_id)).map(|e| ConvMember { user_id: e.client_id, name: e.name, role: Role::Client }));
    }
    Ok(out)
}

/// `GET /conversations/people`: who the caller can message.
pub async fn people(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<ConvMember>>> {
    Ok(Json(reachable(&st, &ctx).await?))
}

/// `GET /conversations`
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let all = mine(&st, &ctx).await?;
    let total: u32 = all.iter().map(|c| c.unread_for(ctx.user_id())).sum();
    Ok(Json(json!({ "unreadTotal": total, "conversations": all.iter().map(|c| view(&ctx, c)).collect::<Vec<_>>() })))
}

/// The people named in `ids`, each one the caller may add (or, when
/// editing a group, already in it).
async fn pick(st: &AppState, ctx: &Ctx, ids: &[String], already: &[ConvMember]) -> ApiResult<Vec<ConvMember>> {
    let allowed = reachable(st, ctx).await?;
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in ids.iter().filter(|id| *id != ctx.user_id()) {
        if !seen.insert(id.as_str()) {
            continue;
        }
        let m = allowed.iter().chain(already).find(|m| &m.user_id == id).ok_or(ApiError::Forbidden("you can only add your teammates and the clients you message"))?;
        out.push(m.clone());
    }
    if out.is_empty() {
        return Err(bad_request("pick who to message"));
    }
    if out.len() + 1 > MAX_MEMBERS {
        return Err(bad_request(format!("a group can have at most {MAX_MEMBERS} people; use a broadcast to message more clients")));
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct NewConversation {
    members: Vec<String>,
    #[serde(default)]
    name: Option<String>,
}

/// `POST /conversations`: one teammate (and no name) opens your direct
/// conversation with them; anything else starts a group.
pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(body): Json<NewConversation>) -> ApiResult<Json<Value>> {
    let others = pick(&st, &ctx, &body.members, &[]).await?;
    let head = head(&ctx)?.to_string();
    let named = body.name.as_deref().is_some_and(|n| !n.trim().is_empty());
    let mut members = vec![me(&ctx)];
    if let [one] = others.as_slice() {
        if !named && one.role == Role::Client {
            return Err(bad_request(format!("message {} in your private conversation with them, or name the group", one.name)));
        }
        if !named {
            let id = direct_id(ctx.user_id(), &one.user_id);
            let (pk, sk) = summary_key(&head, &id);
            if let Some(c) = st.store.get::<Conversation>(&pk, &sk).await? {
                return Ok(Json(view(&ctx, &c)));
            }
            members.push(one.clone());
            let mut c = Conversation {
                id, kind: ConvKind::Direct, name: String::new(), members: vec![], created_by: ctx.user_id().to_string(), created_at: Utc::now(),
                last_at: None, last_text: String::new(), last_author: String::new(), unread: Default::default(),
            };
            c.set_members(members);
            // Both may start it at once; the first one wins.
            if !st.store.put_new(&pk, &sk, &c).await? {
                c = st.store.get(&pk, &sk).await?.ok_or(ApiError::NotFound("conversation"))?;
            }
            return Ok(Json(view(&ctx, &c)));
        }
    }
    members.extend(others);
    let name = group_name(body.name.as_deref(), &members, ctx.user_id()).map_err(bad_request)?;
    let mut c = Conversation {
        id: new_id(), kind: ConvKind::Group, name, members: vec![], created_by: ctx.user_id().to_string(), created_at: Utc::now(),
        last_at: None, last_text: String::new(), last_author: String::new(), unread: Default::default(),
    };
    c.set_members(members);
    let (pk, sk) = summary_key(&head, &c.id);
    st.store.put(&pk, &sk, &c).await?;
    Ok(Json(view(&ctx, &c)))
}

/// `GET /conversations/{id}`: the conversation and its latest messages,
/// oldest first; photos and voice notes carry a short-lived `attachmentUrl`.
pub async fn get_one(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (_, c) = load(&st, &ctx, &id).await?;
    let mut msgs: Vec<ConvMessage> = st.store.latest(&format!("CONV#{id}"), "MSG#", SHOWN).await?;
    msgs.reverse();
    let msgs = try_join_all(msgs.into_iter().map(|m| {
        let st = &st;
        async move {
            let mut v = serde_json::to_value(&m).map_err(|e| ApiError::Internal(e.to_string()))?;
            if let Some(a) = &m.attachment {
                v["attachmentUrl"] = json!(presign_get(st, &a.key).await?);
            }
            ApiResult::Ok(v)
        }
    }))
    .await?;
    Ok(Json(json!({ "conversation": view(&ctx, &c), "messages": msgs })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAttachment {
    kind: AttachmentKind,
    content_type: String,
    size: i64,
}

/// `POST /conversations/{id}/attachments`: an upload URL for a photo or
/// voice note to send here.
pub async fn attachment(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<NewAttachment>) -> ApiResult<Json<Value>> {
    let (head, c) = load(&st, &ctx, &id).await?;
    let (types, max) = match body.kind {
        AttachmentKind::Photo => (IMAGE_TYPES, MAX_PHOTO_BYTES),
        AttachmentKind::Voice => (AUDIO_TYPES, MAX_VOICE_BYTES),
    };
    let ext = ext_for(&body.content_type, types)?;
    check_size(body.size, max)?;
    let key = format!("{}{}.{ext}", media_prefix(&head, &c.id), new_id());
    let up = presign_put(&st, &key, &body.content_type, body.size, Some(PENDING_TAG)).await?;
    Ok(Json(json!({ "key": key, "uploadUrl": up.url, "uploadHeaders": up.headers })))
}

#[derive(Deserialize)]
pub struct NewMessage {
    #[serde(default)]
    text: String,
    #[serde(default)]
    attachment: Option<Attachment>,
}

/// `POST /conversations/{id}/messages`
pub async fn send(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<NewMessage>) -> ApiResult<Json<ConvMessage>> {
    let (head, c) = load(&st, &ctx, &id).await?;
    let text = body.text.trim();
    if text.chars().count() > MAX_TEXT || (text.is_empty() && body.attachment.is_none()) {
        return Err(bad_request(format!("a message is 1-{MAX_TEXT} characters")));
    }
    if let Some(a) = &body.attachment {
        ext_for(&a.content_type, if a.kind == AttachmentKind::Photo { IMAGE_TYPES } else { AUDIO_TYPES })?;
        if !a.key.starts_with(&media_prefix(&head, &c.id)) || a.key.contains("..") {
            return Err(bad_request("that attachment isn't from this conversation"));
        }
        if a.duration_sec.is_some_and(|d| !(0.0..=MAX_VOICE_SECONDS).contains(&d)) {
            return Err(bad_request("voice notes can be at most 2 minutes"));
        }
        if !s3_object_exists(&st, &a.key).await? {
            return Err(bad_request("the attachment hasn't finished uploading"));
        }
        untag(&st, &a.key).await?;
    }
    let m = ConvMessage {
        id: new_id(),
        author_id: ctx.user_id().to_string(),
        author_name: ctx.display_name(),
        author_role: ctx.profile.role,
        text: text.to_string(),
        attachment: body.attachment,
        created_at: Utc::now(),
    };
    st.store.put(&format!("CONV#{id}"), &format!("MSG#{}#{}", m.created_at.to_rfc3339(), m.id), &m).await?;
    on_message(&st, &head, &c, &m).await;
    Ok(Json(m))
}

/// Records a new message on the summary (last message, everyone else's
/// unread count) and tells everyone else; failures here don't fail the send.
async fn on_message(st: &AppState, head: &str, c: &Conversation, m: &ConvMessage) {
    let others: Vec<&ConvMember> = c.members.iter().filter(|x| x.user_id != m.author_id).collect();
    let preview: String = m.preview_text().chars().take(200).collect();
    let (pk, sk) = summary_key(head, &c.id);
    let mut expr = "SET lastAt = :at, lastText = :t, lastAuthor = :a".to_string();
    let mut req = st.store.ddb.update_item().table_name(&st.store.table).key("pk", s(pk)).key("sk", s(sk));
    for (i, o) in others.iter().enumerate() {
        expr.push_str(&format!(", unread.#m{i} = if_not_exists(unread.#m{i}, :zero) + :one"));
        req = req.expression_attribute_names(format!("#m{i}"), o.user_id.as_str());
    }
    let res = req
        .update_expression(expr)
        .condition_expression("attribute_exists(pk)")
        .expression_attribute_values(":at", s(m.created_at.to_rfc3339()))
        .expression_attribute_values(":t", s(preview))
        .expression_attribute_values(":a", s(m.author_name.as_str()))
        .expression_attribute_values(":zero", AttributeValue::N("0".into()))
        .expression_attribute_values(":one", AttributeValue::N("1".into()))
        .send()
        .await;
    if let Err(e) = res {
        tracing::warn!(error = %aws_sdk_dynamodb::error::DisplayErrorContext(e), "conversation update failed");
    }
    let title = match c.kind {
        ConvKind::Group => format!("{} in {}", m.author_name, c.name),
        ConvKind::Direct => m.author_name.clone(),
    };
    let (url, tag, body) = (format!("#/conversations/{}", c.id), format!("conv-{}", c.id), m.preview_text());
    stream::iter(others)
        .for_each_concurrent(8, |o| {
            let (title, url, tag, body) = (&title, &url, &tag, &body);
            async move {
                if let Some(rt) = &st.realtime {
                    rt.notify(&o.user_id, &json!({ "type": "conversation", "id": c.id })).await;
                }
                super::notify::notify_user(st, &o.user_id, title, body, url, tag).await;
            }
        })
        .await;
}

/// `POST /conversations/{id}/read`: the caller has seen it.
pub async fn mark_read(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (head, c) = load(&st, &ctx, &id).await?;
    let (pk, sk) = summary_key(&head, &c.id);
    st.store
        .ddb
        .update_item()
        .table_name(&st.store.table)
        .key("pk", s(pk))
        .key("sk", s(sk))
        .update_expression("SET unread.#me = :zero")
        .condition_expression("attribute_exists(pk)")
        .expression_attribute_names("#me", ctx.user_id())
        .expression_attribute_values(":zero", AttributeValue::N("0".into()))
        .send()
        .await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct EditGroup {
    #[serde(default)]
    name: Option<String>,
    /// Everyone but the caller, who stays in.
    members: Vec<String>,
}

/// `PUT /conversations/{id}`: rename a group or change who's in it (its
/// starter or the head coach). People already in it may stay; anyone new
/// must be someone the editor could add.
pub async fn update(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<EditGroup>) -> ApiResult<Json<Value>> {
    let (head, mut c) = load(&st, &ctx, &id).await?;
    if !can_manage(&ctx, &c) {
        return Err(ApiError::Forbidden("only the group's starter or the head coach changes it"));
    }
    let mut members = vec![me(&ctx)];
    members.extend(pick(&st, &ctx, &body.members, &c.members).await?);
    c.name = group_name(body.name.as_deref(), &members, ctx.user_id()).map_err(bad_request)?;
    c.set_members(members);
    let (pk, sk) = summary_key(&head, &c.id);
    st.store.put(&pk, &sk, &c).await?;
    Ok(Json(view(&ctx, &c)))
}

/// `POST /conversations/{id}/leave`: out of a group (the last one out
/// deletes it).
pub async fn leave(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (head, mut c) = load(&st, &ctx, &id).await?;
    if c.kind != ConvKind::Group {
        return Err(bad_request("you can only leave a group"));
    }
    let rest: Vec<ConvMember> = c.members.iter().filter(|m| m.user_id != ctx.user_id()).cloned().collect();
    if rest.is_empty() {
        remove_all(&st, &head, &c.id).await?;
    } else {
        c.set_members(rest);
        let (pk, sk) = summary_key(&head, &c.id);
        st.store.put(&pk, &sk, &c).await?;
    }
    Ok(Json(json!({ "left": c.id })))
}

/// `DELETE /conversations/{id}`: a group, its messages and files.
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let (head, c) = load(&st, &ctx, &id).await?;
    if !can_manage(&ctx, &c) {
        return Err(ApiError::Forbidden("only the group's starter or the head coach deletes it"));
    }
    remove_all(&st, &head, &c.id).await?;
    Ok(Json(json!({ "deleted": c.id })))
}

async fn remove_all(st: &AppState, head: &str, id: &str) -> ApiResult<()> {
    super::account::delete_media(st, &media_prefix(head, id)).await?;
    super::account::delete_partition(st, &format!("CONV#{id}")).await?;
    let (pk, sk) = summary_key(head, id);
    st.store.delete(&pk, &sk).await
}

/// Takes `user_id` out of `head`'s groups: a client who left the coach (or
/// deleted their account, when `erase` also deletes what they sent), or an
/// assistant who left the team. Direct conversations stay as history.
pub async fn forget(st: &AppState, head: &str, user_id: &str, erase: bool) -> ApiResult<()> {
    let all: Vec<Conversation> = st.store.query_prefix(&format!("COACH#{head}"), "CONV#").await?;
    for mut c in all.into_iter().filter(|c| c.kind == ConvKind::Group && c.member(user_id).is_some()) {
        if erase {
            let pk = format!("CONV#{}", c.id);
            let theirs: Vec<ConvMessage> = st.store.query_prefix(&pk, "MSG#").await?;
            let theirs: Vec<ConvMessage> = theirs.into_iter().filter(|m| m.author_id == user_id).collect();
            for m in theirs.iter().filter_map(|m| m.attachment.as_ref()) {
                let _ = st.s3.delete_object().bucket(&st.bucket).key(&m.key).send().await;
            }
            st.store.batch_delete(theirs.iter().map(|m| (pk.clone(), format!("MSG#{}#{}", m.created_at.to_rfc3339(), m.id))).collect()).await?;
        }
        let rest: Vec<ConvMember> = c.members.iter().filter(|m| m.user_id != user_id).cloned().collect();
        if rest.is_empty() {
            remove_all(st, head, &c.id).await?;
        } else {
            c.set_members(rest);
            let (pk, sk) = summary_key(head, &c.id);
            st.store.put(&pk, &sk, &c).await?;
        }
    }
    Ok(())
}

/// What a client sent in groups, for their data export.
pub async fn sent_by(st: &AppState, head: &str, user_id: &str) -> ApiResult<Vec<Value>> {
    let all: Vec<Conversation> = st.store.query_prefix(&format!("COACH#{head}"), "CONV#").await?;
    let mut out = Vec::new();
    for c in all.iter().filter(|c| c.member(user_id).is_some()) {
        let msgs: Vec<ConvMessage> = st.store.query_prefix(&format!("CONV#{}", c.id), "MSG#").await?;
        for m in msgs.into_iter().filter(|m| m.author_id == user_id) {
            out.push(json!({ "group": c.name, "message": m }));
        }
    }
    Ok(out)
}

#[derive(Deserialize)]
pub struct Broadcast {
    clients: Vec<String>,
    text: String,
}

/// `POST /broadcasts`: the same message to each client's private thread.
pub async fn broadcast(State(st): State<AppState>, ctx: Ctx, Json(body): Json<Broadcast>) -> ApiResult<Json<Value>> {
    ctx.require(Perm::Messaging)?;
    let text = body.text.trim();
    if text.is_empty() || text.chars().count() > MAX_TEXT {
        return Err(bad_request(format!("a message is 1-{MAX_TEXT} characters")));
    }
    let mut seen = HashSet::new();
    let ids: Vec<&String> = body.clients.iter().filter(|c| seen.insert(c.as_str())).collect();
    if ids.is_empty() || ids.len() > MAX_BROADCAST {
        return Err(bad_request(format!("pick 1-{MAX_BROADCAST} clients")));
    }
    // Every client first, so one the caller can't message sends nothing.
    let mut scopes = Vec::new();
    for cid in ids {
        scopes.push(client_scope_as(&st, &ctx, cid, &[Perm::Messaging]).await?);
    }
    let sent = scopes.len();
    stream::iter(scopes)
        .map(|scope| {
            let (st, ctx) = (&st, &ctx);
            async move {
                let c = Comment {
                    id: new_id(),
                    author_id: ctx.user_id().to_string(),
                    author_role: ctx.profile.role,
                    author_name: ctx.display_name(),
                    log_id: None,
                    item_id: None,
                    set_index: None,
                    text: text.to_string(),
                    reply_id: None,
                    attachment: None,
                    video_id: None,
                    about: None,
                    created_at: Utc::now(),
                };
                st.store.put(&scope.pk(), &format!("COMMENT#{}#{}", c.created_at.to_rfc3339(), c.id), &c).await?;
                super::messages::on_message(st, &scope, &c).await;
                ApiResult::Ok(())
            }
        })
        .buffer_unordered(8)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .collect::<ApiResult<Vec<()>>>()?;
    Ok(Json(json!({ "sent": sent })))
}
