//! Message threads (MS-1): one conversation per coach-client pair. The
//! messages are the client's `COMMENT#` items (routes/comments.rs); each
//! thread's summary, `COACH#{coach}` / `THREAD#{client}`, holds the last
//! message and each side's unread count, so the coach's inbox is one query.
//! A new message is also published to the recipient's real-time inbox
//! (realtime.rs), so open apps refresh (or poll when they can't connect),
//! and pushed to their devices as a notification (routes/notify.rs).

use aws_sdk_dynamodb::types::AttributeValue;
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use futures::stream::{self, StreamExt};
use ptcore::model::{Comment, MessageThread, Role, RosterEntry};
use serde_json::{json, Value};

use ptcore::team::Perm;
use crate::auth::{client_scope_as, ClientScope, Ctx};
use crate::error::ApiResult;
use crate::store::s;
use crate::util::audit_client;
use crate::AppState;

/// Most new messages returned per thread in the inbox.
const NEW_SHOWN: usize = 5;

fn thread_key(coach: &str, client: &str) -> (String, String) {
    (format!("COACH#{coach}"), format!("THREAD#{client}"))
}

fn preview(text: &str) -> String {
    let t: String = text.chars().take(200).collect();
    if t.len() < text.len() { format!("{t}…") } else { t }
}

/// Records a new message on its thread (last message, the recipient's unread
/// count) and tells the recipient's open apps. Called after the message is
/// stored; failures here don't fail the send.
pub async fn on_message(st: &AppState, scope: &ClientScope, c: &Comment) {
    let coach = &scope.client.coach_id;
    let client = &scope.client.user_id;
    let (pk, sk) = thread_key(coach, client);
    let (unread, recipient) = if c.author_role == Role::Client { ("unreadCoach", coach) } else { ("unreadClient", client) };
    let res = st
        .store
        .ddb
        .update_item()
        .table_name(&st.store.table)
        .key("pk", s(pk))
        .key("sk", s(sk))
        .update_expression(format!(
            "SET clientId = :c, clientName = :n, lastAt = :at, lastText = :t, lastAuthorRole = :r ADD {unread} :one"
        ))
        .expression_attribute_values(":c", s(client.as_str()))
        .expression_attribute_values(":n", s(scope.client.name.as_str()))
        .expression_attribute_values(":at", s(c.created_at.to_rfc3339()))
        .expression_attribute_values(":t", s(preview(&c.preview_text())))
        .expression_attribute_values(":r", s(if c.author_role == Role::Client { "client" } else { "coach" }))
        .expression_attribute_values(":one", AttributeValue::N("1".into()))
        .send()
        .await;
    if let Err(e) = res {
        tracing::warn!(error = %aws_sdk_dynamodb::error::DisplayErrorContext(e), "thread update failed");
    }
    if let Some(rt) = &st.realtime {
        rt.notify(recipient, &json!({ "type": "message", "clientId": client })).await;
    }
    // A push notification too, for when the app isn't open; tapping it opens the thread.
    let url = if c.author_role == Role::Client { format!("#/c/{client}/messages") } else { "#/coach".to_string() };
    super::notify::notify_user(st, recipient, &c.author_name, &c.preview_text(), &url, &format!("thread-{client}")).await;
    // Assistants who handle this client's messages hear about it too.
    if c.author_role == Role::Client {
        let team: Vec<ptcore::team::Member> = st.store.query_prefix(&format!("COACH#{coach}"), "ASSISTANT#").await.unwrap_or_default();
        for m in team.iter().filter(|m| m.assigned(client) && m.can(ptcore::team::Perm::Messaging)) {
            if let Some(rt) = &st.realtime {
                rt.notify(&m.user_id, &json!({ "type": "message", "clientId": client })).await;
            }
            super::notify::notify_user(st, &m.user_id, &c.author_name, &c.preview_text(), &url, &format!("thread-{client}")).await;
        }
    }
}

/// `POST /clients/{cid}/messages/read`: the caller has seen the conversation.
pub async fn mark_read(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Messaging]).await?;
    let (pk, sk) = thread_key(&scope.client.coach_id, &cid);
    let (unread, read_at) = if scope.by_coach { ("unreadCoach", "coachReadAt") } else { ("unreadClient", "clientReadAt") };
    st.store
        .ddb
        .update_item()
        .table_name(&st.store.table)
        .key("pk", s(pk))
        .key("sk", s(sk))
        .update_expression(format!("SET clientId = :c, {unread} = :zero, {read_at} = :now"))
        .expression_attribute_values(":c", s(cid.as_str()))
        .expression_attribute_values(":zero", AttributeValue::N("0".into()))
        .expression_attribute_values(":now", s(Utc::now().to_rfc3339()))
        .send()
        .await?;
    Ok(Json(json!({ "ok": true })))
}

/// A thread built from a client's existing messages, for conversations from
/// before thread summaries existed. Unread is the coach's guess: the client's
/// messages since the coach last wrote.
async fn backfill(st: &AppState, coach: &str, e: &RosterEntry) -> ApiResult<MessageThread> {
    let recent: Vec<Comment> = st.store.latest(&format!("CLIENT#{}", e.client_id), "COMMENT#", 50).await?;
    let mut t = MessageThread { client_id: e.client_id.clone(), client_name: e.name.clone(), ..Default::default() };
    if let Some(last) = recent.first() {
        t.last_at = Some(last.created_at);
        t.last_text = preview(&last.text);
        t.last_author_role = Some(last.author_role);
        t.unread_coach = recent.iter().take_while(|c| c.author_role == Role::Client).count() as u32;
        let (pk, sk) = thread_key(coach, &e.client_id);
        st.store.put(&pk, &sk, &t).await?;
    }
    Ok(t)
}

/// `GET /messages`: the caller's client conversations, newest first, each
/// with its unread count; for the coach, also the unread messages themselves
/// (the latest few), so the inbox can show them. A client has one
/// conversation. `conversationsUnread` counts team and group messages
/// (routes/conversations.rs) for the badge.
pub async fn inbox(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let groups: u32 = super::conversations::mine(&st, &ctx).await?.iter().map(|c| c.unread_for(ctx.user_id())).sum();
    if ctx.profile.role == Role::Client {
        let (pk, sk) = thread_key(&ctx.profile.coach_id, ctx.user_id());
        let t: MessageThread = st.store.get(&pk, &sk).await?.unwrap_or_default();
        return Ok(Json(json!({ "unreadTotal": t.unread_client, "conversationsUnread": groups, "threads": [] })));
    }
    // Every teammate has team conversations; clients' threads need Messaging.
    if ctx.is_assistant() && ctx.require(Perm::Messaging).is_err() {
        return Ok(Json(json!({ "unreadTotal": 0, "conversationsUnread": groups, "threads": [] })));
    }
    let coach = ctx.require(Perm::Messaging)?.to_string();
    let pk = format!("COACH#{coach}");
    let mut roster: Vec<RosterEntry> = st.store.query_prefix(&pk, "CLIENT#").await?;
    roster.retain(|e| ctx.sees_client(&e.client_id));
    let threads: Vec<MessageThread> = st.store.query_prefix(&pk, "THREAD#").await?;

    // Every client gets a row (so the coach can start a conversation), with
    // their current name from the roster.
    let rows: Vec<ApiResult<Value>> = stream::iter(roster)
        .map(|e| {
            let st = st.clone();
            let coach = coach.clone();
            let found = threads.iter().find(|t| t.client_id == e.client_id).cloned();
            let ctx = ctx.clone();
            async move {
                let t = match found {
                    Some(t) => t,
                    None => backfill(&st, &coach, &e).await?,
                };
                let new_messages: Vec<Comment> = if t.unread_coach > 0 {
                    let client_pk = format!("CLIENT#{}", e.client_id);
                    audit_client(&st, &ctx, &client_pk, "view-messages").await;
                    let mut recent: Vec<Comment> = st.store.latest(&client_pk, "COMMENT#", 20).await?;
                    recent.retain(|c| c.author_role == Role::Client);
                    recent.truncate((t.unread_coach as usize).min(NEW_SHOWN));
                    recent.reverse();
                    recent
                } else {
                    vec![]
                };
                Ok(json!({
                    "clientId": e.client_id,
                    "clientName": e.name,
                    "unread": t.unread_coach,
                    "lastAt": t.last_at,
                    "lastText": t.last_text,
                    "lastAuthorRole": t.last_author_role,
                    "newMessages": new_messages,
                }))
            }
        })
        .buffer_unordered(8)
        .collect()
        .await;
    let mut rows = rows.into_iter().collect::<ApiResult<Vec<Value>>>()?;
    // Unread first, then most recent; clients with no messages last, by name.
    rows.sort_by(|a, b| {
        let key = |v: &Value| (v["unread"].as_u64() == Some(0), std::cmp::Reverse(v["lastAt"].as_str().unwrap_or("").to_string()), v["clientName"].as_str().unwrap_or("").to_lowercase());
        key(a).cmp(&key(b))
    });
    let total: u64 = rows.iter().filter_map(|r| r["unread"].as_u64()).sum();
    Ok(Json(json!({ "unreadTotal": total, "conversationsUnread": groups, "threads": rows })))
}

#[cfg(test)]
mod tests {
    #[test]
    fn previews_are_cut_at_200_characters() {
        assert_eq!(super::preview("hi"), "hi");
        let long = "é".repeat(250);
        let p = super::preview(&long);
        assert_eq!(p.chars().count(), 201);
        assert!(p.ends_with('…'));
    }
}
