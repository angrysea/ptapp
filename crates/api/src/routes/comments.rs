//! FB-1: comments on a whole workout or a single exercise/set, and the
//! coach-client message thread (MS-1). A message can carry a photo or a voice
//! note: the browser gets an upload URL from `attachment`, uploads, then sends
//! the message naming the uploaded key, which is checked and confirmed here.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use futures::future::try_join_all;
use ptcore::model::{Attachment, AttachmentKind, Comment};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiResult};
use crate::util::{
    audit, check_size, ext_for, new_id, presign_get, presign_put, s3_object_exists, untag, AUDIO_TYPES, IMAGE_TYPES, MAX_PHOTO_BYTES,
    MAX_VOICE_BYTES, PENDING_TAG,
};
use crate::AppState;

/// Longest voice note, in seconds.
const MAX_VOICE_SECONDS: f64 = 125.0;

fn attachment_prefix(scope: &crate::auth::ClientScope) -> String {
    format!("media/{}/{}/messages/", scope.client.coach_id, scope.client.user_id)
}

/// The thread, oldest first; messages with a photo or voice note carry a
/// short-lived `attachmentUrl`.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Vec<Value>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-comments").await;
    let comments: Vec<Comment> = st.store.query_prefix(&scope.pk(), "COMMENT#").await?;
    let out = try_join_all(comments.into_iter().map(|c| {
        let st = &st;
        async move {
            let mut v = serde_json::to_value(&c).map_err(|e| crate::error::ApiError::Internal(e.to_string()))?;
            if let Some(a) = &c.attachment {
                v["attachmentUrl"] = json!(presign_get(st, &a.key).await?);
            }
            ApiResult::Ok(v)
        }
    }))
    .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAttachment {
    kind: AttachmentKind,
    content_type: String,
    size: i64,
}

/// `POST /clients/{cid}/attachments`: an upload URL for a photo or voice note
/// to send in the thread. Unsent uploads keep the pending tag and expire.
pub async fn attachment(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<NewAttachment>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let (types, max) = match body.kind {
        AttachmentKind::Photo => (IMAGE_TYPES, MAX_PHOTO_BYTES),
        AttachmentKind::Voice => (AUDIO_TYPES, MAX_VOICE_BYTES),
    };
    let ext = ext_for(&body.content_type, types)?;
    check_size(body.size, max)?;
    let key = format!("{}{}.{ext}", attachment_prefix(&scope), new_id());
    let up = presign_put(&st, &key, &body.content_type, body.size, Some(PENDING_TAG)).await?;
    Ok(Json(json!({ "key": key, "uploadUrl": up.url, "uploadHeaders": up.headers })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    #[serde(default)]
    text: String,
    log_id: Option<String>,
    item_id: Option<String>,
    set_index: Option<u32>,
    attachment: Option<Attachment>,
}

pub async fn create(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<NewComment>,
) -> ApiResult<Json<Comment>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let text = body.text.trim();
    if text.chars().count() > 4000 || (text.is_empty() && body.attachment.is_none()) {
        return Err(bad_request("comment must be 1-4000 characters"));
    }
    // An attachment must be one uploaded for this thread, and finished.
    if let Some(a) = &body.attachment {
        let allowed = match a.kind {
            AttachmentKind::Photo => IMAGE_TYPES,
            AttachmentKind::Voice => AUDIO_TYPES,
        };
        ext_for(&a.content_type, allowed)?;
        if !a.key.starts_with(&attachment_prefix(&scope)) || a.key.contains("..") {
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
    let c = Comment {
        id: new_id(),
        author_id: ctx.user_id().to_string(),
        author_role: ctx.profile.role,
        author_name: ctx.profile.name.clone(),
        log_id: body.log_id,
        item_id: body.item_id,
        set_index: body.set_index,
        text: text.to_string(),
        reply_id: None,
        attachment: body.attachment,
        created_at: Utc::now(),
    };
    st.store
        .put(&scope.pk(), &format!("COMMENT#{}#{}", c.created_at.to_rfc3339(), c.id), &c)
        .await?;
    super::messages::on_message(&st, &scope, &c).await;
    Ok(Json(c))
}
