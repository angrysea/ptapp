//! A client's own data (privacy NFR): download everything stored about them
//! (`GET /me/export`), or delete their account for good (`DELETE /me/account`).
//! Coaches' and admins' accounts hold other people's work (clients, coach
//! invitations), so those are closed by the admin instead.

use aws_sdk_s3::types::{Delete, ObjectIdentifier};
use axum::extract::State;
use axum::Json;
use chrono::Utc;
use ptcore::model::{Profile, ReviewEntry, Role};
use serde::Deserialize;
use serde_json::{json, Value};
use serde_dynamo::aws_sdk_dynamodb_1::from_item;

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
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

fn media_prefix(p: &Profile) -> String {
    format!("media/{}/{}/", p.coach_id, p.user_id)
}

fn sk_of(item: &Item) -> String {
    item.get("sk").and_then(|v| v.as_s().ok()).cloned().unwrap_or_default()
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
    let keys = media_keys(&st, &media_prefix(p)).await?;
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
    let keys = media_keys(&st, &media_prefix(&p)).await?;
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

    // 2. Everything in the client's partition.
    let client_pk = format!("CLIENT#{}", p.user_id);
    let mine: Vec<(String, String)> = st.store.partition(&client_pk).await?.iter().map(|i| (client_pk.clone(), sk_of(i))).collect();
    st.store.batch_delete(mine).await?;

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
    st.store.batch_delete(theirs).await?;

    // 4. Their user records: profile, push subscriptions (and which endpoints they owned).
    let user_pk = format!("USER#{}", p.user_id);
    let mut user_keys = Vec::new();
    for item in st.store.partition(&user_pk).await? {
        let sk = sk_of(&item);
        if let Some(endpoint) = sk.strip_prefix("PUSH#") {
            user_keys.push((format!("PUSHEP#{endpoint}"), "OWNER".to_string()));
        }
        user_keys.push((user_pk.clone(), sk));
    }
    st.store.batch_delete(user_keys).await?;

    // 5. Their sign-in.
    if let Some(cog) = &st.cognito {
        cog.client
            .admin_delete_user()
            .user_pool_id(&cog.pool_id)
            .username(&ctx.caller.sub)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("cognito delete: {}", aws_sdk_cognitoidentityprovider::error::DisplayErrorContext(e))))?;
    }
    tracing::info!(user = %p.user_id, coach = %p.coach_id, "client account deleted");
    Ok(Json(json!({ "deleted": true })))
}
