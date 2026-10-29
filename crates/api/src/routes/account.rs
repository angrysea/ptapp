//! Closing accounts (privacy NFR). A client downloads everything stored
//! about them (`GET /me/export`) or deletes their own account
//! (`DELETE /me/account`). Coaches' accounts hold their clients' programs and
//! conversations, so an admin suspends or deletes those instead
//! (`/admin/coaches/{id}/suspended`, `DELETE /admin/coaches/{id}`).

use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{CoachEntry, Profile, ReviewEntry, Role, RosterEntry};
use ptcore::sessions::{Session, SessionStatus};
use serde::Deserialize;
use serde_json::{json, Value};
use serde_dynamo::aws_sdk_dynamodb_1::from_item;

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::coaches;
use crate::routes::invites::ADMIN_PK;
use crate::store::Item;
use crate::util::presign_get;
use crate::AppState;

/// Most files listed (with download links) in one export.
const MAX_EXPORT_FILES: usize = 1000;

fn client_only(ctx: &Ctx) -> ApiResult<()> {
    if ctx.profile.role != Role::Client {
        return Err(ApiError::Forbidden("coach and admin accounts are closed by the PT Coach admin; contact support"));
    }
    Ok(())
}

/// The client's media folders: under their coach, and any earlier coaches.
fn media_prefixes(p: &Profile) -> Vec<String> {
    std::iter::once(&p.coach_id).chain(&p.former_coach_ids).map(|c| format!("media/{c}/{}/", p.user_id)).collect()
}

fn sk_of(item: &Item) -> String {
    item.get("sk").and_then(|v| v.as_s().ok()).cloned().unwrap_or_default()
}

fn cognito_error<E: std::error::Error + Send + Sync + 'static, R: std::fmt::Debug>(
    what: &str,
    e: aws_sdk_cognitoidentityprovider::error::SdkError<E, R>,
) -> ApiError {
    ApiError::Internal(format!("cognito {what}: {}", aws_sdk_cognitoidentityprovider::error::DisplayErrorContext(e)))
}

/// Every file under `prefix` in the media bucket.
async fn media_keys(st: &AppState, prefix: &str) -> ApiResult<Vec<String>> {
    let mut keys = Vec::new();
    let mut token = None;
    loop {
        let page = st
            .s3
            .list_objects_v2()
            .bucket(&st.bucket)
            .prefix(prefix)
            .set_continuation_token(token)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("s3 list: {e}")))?;
        keys.extend(page.contents().iter().filter_map(|o| o.key().map(str::to_owned)));
        match page.next_continuation_token() {
            Some(t) => token = Some(t.to_owned()),
            None => break,
        }
    }
    Ok(keys)
}

/// Deletes every file under `prefix` in the media bucket.
async fn delete_media(st: &AppState, prefix: &str) -> ApiResult<()> {
    let keys = media_keys(st, prefix).await?;
    for chunk in keys.chunks(1000) {
        let objects: Vec<ObjectIdentifier> = chunk.iter().filter_map(|k| ObjectIdentifier::builder().key(k).build().ok()).collect();
        let delete = Delete::builder().set_objects(Some(objects)).quiet(true).build().map_err(|e| ApiError::Internal(e.to_string()))?;
        st.s3
            .delete_objects()
            .bucket(&st.bucket)
            .delete(delete)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("s3 delete: {e}")))?;
    }
    Ok(())
}

/// Deletes a whole partition.
async fn delete_partition(st: &AppState, pk: &str) -> ApiResult<()> {
    let keys = st.store.partition(pk).await?.iter().map(|i| (pk.to_string(), sk_of(i))).collect();
    st.store.batch_delete(keys).await
}

/// The user's push subscriptions (and which device endpoints they own).
async fn delete_push(st: &AppState, user_id: &str) -> ApiResult<()> {
    let user_pk = format!("USER#{user_id}");
    let mut keys = Vec::new();
    for item in st.store.partition(&user_pk).await? {
        let sk = sk_of(&item);
        if let Some(endpoint) = sk.strip_prefix("PUSH#") {
            keys.push((format!("PUSHEP#{endpoint}"), "OWNER".to_string()));
            keys.push((user_pk.clone(), sk));
        }
    }
    st.store.batch_delete(keys).await
}

/// The user's records (profile, push subscriptions) and their sign-in.
async fn delete_user(st: &AppState, user_id: &str) -> ApiResult<()> {
    delete_push(st, user_id).await?;
    super::sessions::forget_calendar(st, user_id).await?;
    delete_partition(st, &format!("USER#{user_id}")).await?;
    if let Some(cog) = &st.cognito {
        cog.client
            .admin_delete_user()
            .user_pool_id(&cog.pool_id)
            .username(user_id)
            .send()
            .await
            .map_err(|e| cognito_error("delete", e))?;
    }
    Ok(())
}

/// `GET /me/export`: the client's profile and every record about them, as
/// JSON, plus download links (valid 15 minutes) for their photos, videos,
/// voice notes and their coach's video replies.
pub async fn export(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    client_only(&ctx)?;
    let p = &ctx.profile;
    let mut records = Vec::new();
    for item in st.store.partition(&format!("CLIENT#{}", p.user_id)).await? {
        // Internal bookkeeping (log pointers) isn't the person's data.
        if sk_of(&item).starts_with("LOGREF#") {
            continue;
        }
        let mut v: Value = from_item(item)?;
        if let Some(o) = v.as_object_mut() {
            o.remove("pk");
            let record = o.remove("sk").unwrap_or(Value::Null);
            o.insert("record".into(), record);
        }
        records.push(v);
    }
    let mut keys = Vec::new();
    for prefix in media_prefixes(p) {
        keys.extend(media_keys(&st, &prefix).await?);
    }
    let mut files = Vec::new();
    for key in keys.iter().take(MAX_EXPORT_FILES) {
        files.push(json!({ "key": key, "url": presign_get(&st, key).await? }));
    }
    Ok(Json(json!({
        "exportedAt": Utc::now(),
        "account": { "email": ctx.caller.email, "profile": p },
        "records": records,
        "files": files,
        "filesNote": "The file links work for 15 minutes from the export time.",
    })))
}

#[derive(Deserialize)]
pub struct ConfirmDelete {
    /// The account's email, typed by the person, so a stray tap can't delete it.
    confirm: String,
}

/// `DELETE /me/account`: deletes the client's account and everything about them:
/// their records, files and conversation, their place on the coach's roster
/// and review queue, their push subscriptions, and their sign-in.
pub async fn delete(State(st): State<AppState>, ctx: Ctx, Json(body): Json<ConfirmDelete>) -> ApiResult<Json<Value>> {
    client_only(&ctx)?;
    let p = ctx.profile.clone();
    if !body.confirm.trim().eq_ignore_ascii_case(&p.email) && !body.confirm.trim().eq_ignore_ascii_case(&ctx.caller.email) {
        return Err(bad_request("type your email address to confirm"));
    }

    // 1. Files: photos, set videos, replies, message attachments.
    for prefix in media_prefixes(&p) {
        delete_media(&st, &prefix).await?;
    }

    // 2. Everything in the client's partition.
    delete_partition(&st, &format!("CLIENT#{}", p.user_id)).await?;

    // 3. Their place in the coach's lists: roster, thread, review queue.
    let coach_pk = format!("COACH#{}", p.coach_id);
    let mut theirs = vec![(coach_pk.clone(), format!("CLIENT#{}", p.user_id)), (coach_pk.clone(), format!("THREAD#{}", p.user_id))];
    let queue: Vec<Item> = st.store.partition(&coach_pk).await?;
    for item in queue.iter().filter(|i| sk_of(i).starts_with("REVIEW#")) {
        let e: ReviewEntry = from_item(item.clone())?;
        if e.client_id == p.user_id {
            theirs.push((coach_pk.clone(), sk_of(item)));
        }
    }
    // ...and the coach's copies of their sessions, with the slot locks.
    for item in queue.iter().filter(|i| sk_of(i).starts_with("SESSION#")) {
        let s: Session = from_item(item.clone())?;
        if s.client_id == p.user_id {
            theirs.push((coach_pk.clone(), sk_of(item)));
            theirs.push((coach_pk.clone(), format!("SLOTLOCK#{}", s.start.to_rfc3339())));
        }
    }
    st.store.batch_delete(theirs).await?;

    // 4. Their user records, push subscriptions and sign-in.
    delete_user(&st, &p.user_id).await?;
    tracing::info!(user = %p.user_id, coach = %p.coach_id, "client account deleted");
    Ok(Json(json!({ "deleted": true })))
}

// ---------------------------------------------------------------- admin: coaches

async fn load_coach(st: &AppState, id: &str) -> ApiResult<Profile> {
    st.store
        .get::<Profile>(&format!("USER#{id}"), "PROFILE")
        .await?
        .filter(|p| p.role == Role::Coach)
        .ok_or(ApiError::NotFound("coach not found"))
}

#[derive(Deserialize)]
pub struct Suspend {
    suspended: bool,
}

/// `PUT /admin/coaches/{id}/suspended`: a suspended coach can't sign in or
/// use the API, gets no notifications and leaves the directory. Their
/// clients keep their accounts and can still log workouts. Lifting the
/// suspension restores all of it.
pub async fn suspend_coach(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<Suspend>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let mut coach = load_coach(&st, &id).await?;
    let at = body.suspended.then(Utc::now);
    coach.suspended_at = at;
    st.store.put(&format!("USER#{id}"), "PROFILE", &coach).await?;
    if let Some(mut entry) = st.store.get::<CoachEntry>(ADMIN_PK, &format!("COACH#{id}")).await? {
        entry.suspended_at = at;
        st.store.put(ADMIN_PK, &format!("COACH#{id}"), &entry).await?;
    }
    if let Some(profile) = coaches::load(&st, &id).await? {
        coaches::sync_directory(&st, &id, &profile).await?;
    }
    if body.suspended {
        delete_push(&st, &id).await?;
        super::sessions::forget_calendar(&st, &id).await?;
    }
    // Disabling the sign-in also ends their sessions: refresh tokens stop
    // working, and the API refuses the access token they still hold.
    if let Some(cog) = &st.cognito {
        if body.suspended {
            cog.client.admin_disable_user().user_pool_id(&cog.pool_id).username(&id).send().await.map_err(|e| cognito_error("disable", e))?;
        } else {
            cog.client.admin_enable_user().user_pool_id(&cog.pool_id).username(&id).send().await.map_err(|e| cognito_error("enable", e))?;
        }
    }
    tracing::info!(coach = %id, by = %ctx.user_id(), suspended = body.suspended, "coach suspension changed");
    Ok(Json(json!({ "suspended": body.suspended })))
}

/// `DELETE /admin/coaches/{id}`: deletes a coach who has no clients left,
/// with everything they made: exercises, programs, invitations, profile,
/// directory entry, files and sign-in. A coach with clients is refused:
/// those clients' training lives under the coach (suspend them instead).
pub async fn delete_coach(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(body): Json<ConfirmDelete>,
) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let coach = load_coach(&st, &id).await?;
    if !body.confirm.trim().eq_ignore_ascii_case(&coach.email) {
        return Err(bad_request("type the coach's email address to confirm"));
    }
    let coach_pk = format!("COACH#{id}");
    let items = st.store.partition(&coach_pk).await?;
    let clients = items.iter().filter(|i| sk_of(i).starts_with("CLIENT#")).count();
    if clients > 0 {
        return Err(ApiError::Conflict(json!({
            "message": format!("this coach still has {clients} client{}; move them to another coach first (or suspend this coach instead)", if clients == 1 { "" } else { "s" }),
            "clients": clients,
        })));
    }

    // 1. Files: exercise demos, profile photo.
    delete_media(&st, &format!("media/{id}/")).await?;

    // 2. Their invitations (each also has its own record), then the whole
    //    coach partition: exercises, programs, threads, review queue.
    let invites: Vec<(String, String)> = items
        .iter()
        .filter_map(|i| sk_of(i).strip_prefix("INVITE#").map(|code| (format!("INVITE#{code}"), "INVITE".to_string())))
        .collect();
    st.store.batch_delete(invites).await?;
    delete_partition(&st, &coach_pk).await?;

    // 3. The directory and the admin's coach list.
    st.store.batch_delete(vec![(coaches::DIRECTORY.to_string(), coach_pk.clone()), (ADMIN_PK.to_string(), coach_pk)]).await?;

    // 4. Their user records, push subscriptions and sign-in.
    delete_user(&st, &id).await?;
    tracing::info!(coach = %id, by = %ctx.user_id(), "coach account deleted");
    Ok(Json(json!({ "deleted": true })))
}

// ---------------------------------------------------------------- admin: moving a client

/// `GET /admin/coaches/{id}/clients`: a coach's roster, for moving clients.
pub async fn coach_clients(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Vec<RosterEntry>>> {
    ctx.require_admin()?;
    let mut list: Vec<RosterEntry> = st.store.query_prefix(&format!("COACH#{id}"), "CLIENT#").await?;
    list.sort_by_key(|r| r.name.to_lowercase());
    Ok(Json(list))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Move {
    coach_id: String,
}

/// `POST /admin/clients/{cid}/move`: the client trains with another coach
/// from now on. Their own data (workouts, measurements, photos, videos,
/// messages, scheduled programs) stays with them; unreviewed videos go to
/// the new coach's queue; sessions still to come with the old coach are
/// cancelled; the old coach's suggestions and alerts about them go.
pub async fn move_client(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<Move>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let mut client: Profile = st
        .store
        .get::<Profile>(&format!("USER#{cid}"), "PROFILE")
        .await?
        .filter(|p| p.role == Role::Client)
        .ok_or(ApiError::NotFound("client not found"))?;
    let to = load_coach(&st, &body.coach_id).await?;
    if to.suspended_at.is_some() {
        return Err(bad_request("that coach is suspended"));
    }
    let from = client.coach_id.clone();
    if from == to.user_id {
        return Err(bad_request("the client already trains with that coach"));
    }
    let (old_pk, new_pk) = (format!("COACH#{from}"), format!("COACH#{}", to.user_id));
    let now = Utc::now();

    // 1. The client's profile now points at the new coach.
    if !client.former_coach_ids.contains(&from) {
        client.former_coach_ids.push(from.clone());
    }
    client.coach_id = to.user_id.clone();
    // Only if nothing moved them (or closed the account) since it was read.
    let moved = st
        .store
        .put_if_with(&format!("USER#{cid}"), "PROFILE", &client, "attribute_exists(pk) AND coachId = :from", &[(":from", crate::store::s(&from))])
        .await?;
    if !moved {
        return Err(ApiError::Conflict(json!({ "message": "the client's account changed meanwhile; reload and try again" })));
    }

    // 2. The rosters.
    let roster = RosterEntry { client_id: cid.clone(), name: client.name.clone(), email: client.email.clone(), joined_at: now };
    st.store.put(&new_pk, &format!("CLIENT#{cid}"), &roster).await?;

    // 3. The old coach's records about them: the queue moves, the rest goes.
    let mut gone = vec![(old_pk.clone(), format!("CLIENT#{cid}")), (old_pk.clone(), format!("THREAD#{cid}"))];
    let mut moved_reviews = Vec::new();
    let mut cancelled = 0;
    for item in st.store.partition(&old_pk).await? {
        let sk = sk_of(&item);
        let about_them = item.get("clientId").and_then(|v| v.as_s().ok()).is_some_and(|c| c == &cid);
        if !about_them {
            continue;
        }
        if sk.starts_with("REVIEW#") {
            let e: ReviewEntry = from_item(item.clone())?;
            moved_reviews.push(crate::store::keyed(&new_pk, &sk, &e)?);
            gone.push((old_pk.clone(), sk));
        } else if sk.starts_with("SESSION#") {
            let mut s: Session = from_item(item.clone())?;
            gone.push((old_pk.clone(), sk.clone()));
            gone.push((old_pk.clone(), format!("SLOTLOCK#{}", s.start.to_rfc3339())));
            if s.start > now && s.status == SessionStatus::Booked {
                s.status = SessionStatus::Cancelled;
                st.store.put(&format!("CLIENT#{cid}"), &sk, &s).await?;
                cancelled += 1;
            }
        } else if sk.starts_with("SUGGEST#") || sk.starts_with("ALERT#") {
            gone.push((old_pk.clone(), sk));
        }
    }
    st.store.batch_put(moved_reviews).await?;
    st.store.batch_delete(gone).await?;

    // 4. Tell the client and the new coach.
    super::notify::notify_user(&st, &cid, &to.name, &format!("You're now training with {}", to.name), "#/coach", "coach-change").await;
    super::notify::notify_user(&st, &to.user_id, &client.name, &format!("{} is now one of your clients", client.name), &format!("#/c/{cid}"), &format!("new-client-{cid}")).await;
    tracing::info!(client = %cid, from = %from, to = %to.user_id, by = %ctx.user_id(), "client moved");
    Ok(Json(json!({ "moved": true, "from": from, "to": to.user_id, "cancelledSessions": cancelled })))
}
