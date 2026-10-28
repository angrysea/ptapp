//! VR-1..VR-3: the coach's reply to a set video. The coach's browser records
//! the clip as they play, slow and step through it, with their voice over it
//! (web/js/views/review.js), and uploads it in parts like a set video
//! (routes/videos.rs helpers). A reply can also be just a written note.
//!
//! Sending a reply (completing its upload, or creating a written one):
//!   * marks the set video reviewed, which takes it out of the review queue
//!     and starts its retention clock; the reply is tagged the same way, so
//!     the lifecycle rule deletes both together;
//!   * posts a message in the coach-client thread that carries the reply, so
//!     the client is told (unread count, live event) and can play it there.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use futures::future::try_join_all;
use ptcore::model::{Comment, Reply, Role, VideoStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::messages::on_message;
use super::videos::{assemble, find, mark_reviewed, part_sizes, sign_parts, start_multipart, tag_reviewed, Complete, PartsQuery, PART_SIZE};
use crate::auth::{client_scope, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{audit, check_size, ext_for, new_id, presign_get, VIDEO_TYPES};
use crate::AppState;

/// About 5 minutes at the ~1.6 Mbps the review screen records, with room to spare.
pub const MAX_REPLY_BYTES: i64 = 200 * 1024 * 1024;
/// Decision from review: replies are capped at 5 minutes (plus encoder rounding).
const MAX_REPLY_SECONDS: f64 = 305.0;
const MAX_NOTE: usize = 4000;

fn reply_sk(id: &str) -> String {
    format!("REPLY#{id}")
}

/// Only the client's coach replies.
async fn coach_scope(st: &AppState, ctx: &Ctx, cid: &str) -> ApiResult<ClientScope> {
    let scope = client_scope(st, ctx, cid).await?;
    if !scope.by_coach {
        return Err(ApiError::Forbidden("only the coach replies to videos"));
    }
    Ok(scope)
}

async fn load(st: &AppState, scope: &ClientScope, rid: &str) -> ApiResult<Reply> {
    st.store.get(&scope.pk(), &reply_sk(rid)).await?.ok_or(ApiError::NotFound("reply"))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewReply {
    /// Absent for a written reply with no recording.
    content_type: Option<String>,
    #[serde(default)]
    size: i64,
    duration_sec: Option<f64>,
    #[serde(default)]
    note: String,
    review_ms: Option<u64>,
}

/// `POST /clients/{cid}/videos/{vid}/replies`: a recording to upload in parts
/// (then `…/complete`), or a written reply, which is sent straight away.
pub async fn create(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, vid)): Path<(String, String)>,
    Json(body): Json<NewReply>,
) -> ApiResult<Json<Value>> {
    let scope = coach_scope(&st, &ctx, &cid).await?;
    let video = find(&st, &scope, &vid).await?;
    if video.status != VideoStatus::Ready {
        return Err(bad_request("this video hasn't finished uploading"));
    }
    let note = body.note.trim().to_string();
    if note.chars().count() > MAX_NOTE {
        return Err(bad_request(format!("the note must be {MAX_NOTE} characters or fewer")));
    }
    let id = new_id();
    let mut reply = Reply {
        id: id.clone(),
        video_id: vid,
        coach_id: ctx.user_id().to_string(),
        status: VideoStatus::Uploading,
        key: String::new(),
        content_type: String::new(),
        size_bytes: 0,
        upload_id: None,
        part_size: PART_SIZE,
        duration_sec: body.duration_sec,
        note,
        review_ms: body.review_ms,
        created_at: Utc::now(),
        sent_at: None,
        seen_at: None,
    };
    let Some(content_type) = body.content_type else {
        if reply.note.is_empty() {
            return Err(bad_request("write a note, or record a reply"));
        }
        finish(&st, &ctx, &scope, &mut reply).await?;
        return Ok(Json(json!({ "reply": reply, "parts": [] })));
    };
    let ext = ext_for(&content_type, VIDEO_TYPES)?;
    check_size(body.size, MAX_REPLY_BYTES)?;
    if body.duration_sec.is_some_and(|s| !(0.0..=MAX_REPLY_SECONDS).contains(&s)) {
        return Err(bad_request("replies can be at most 5 minutes"));
    }
    reply.key = format!("media/{}/{}/replies/{id}.{ext}", scope.client.coach_id, scope.client.user_id);
    reply.upload_id = Some(start_multipart(&st, &reply.key, &content_type).await?);
    reply.content_type = content_type;
    reply.size_bytes = body.size;
    let numbers: Vec<i32> = (1..=part_sizes(reply.size_bytes, reply.part_size).len() as i32).collect();
    let parts = sign_parts(&st, &reply.key, reply.upload_id.as_deref(), reply.size_bytes, reply.part_size, &numbers).await?;
    st.store.put(&scope.pk(), &reply_sk(&reply.id), &reply).await?;
    Ok(Json(json!({ "reply": reply, "parts": parts })))
}

/// Fresh URLs for parts whose URLs expired.
pub async fn parts(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, rid)): Path<(String, String)>,
    Json(body): Json<PartsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = coach_scope(&st, &ctx, &cid).await?;
    let r = load(&st, &scope, &rid).await?;
    let mut numbers = body.parts;
    numbers.sort_unstable();
    numbers.dedup();
    Ok(Json(json!({ "parts": sign_parts(&st, &r.key, r.upload_id.as_deref(), r.size_bytes, r.part_size, &numbers).await? })))
}

/// Assembles the recording and sends the reply. Safe to repeat.
pub async fn complete(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, rid)): Path<(String, String)>,
    Json(body): Json<Complete>,
) -> ApiResult<Json<Reply>> {
    let scope = coach_scope(&st, &ctx, &cid).await?;
    let mut r = load(&st, &scope, &rid).await?;
    if r.status == VideoStatus::Ready {
        return Ok(Json(r));
    }
    let upload_id = r.upload_id.clone().ok_or_else(|| ApiError::Internal("uploading reply without an upload id".into()))?;
    assemble(&st, &r.key, &upload_id, r.size_bytes, r.part_size, &body.parts).await?;
    finish(&st, &ctx, &scope, &mut r).await?;
    Ok(Json(r))
}

/// Sends a reply: the video is marked reviewed, both are tagged for
/// retention, and the thread gets a message carrying the reply. The message
/// reuses the reply's id and time, so a retry rewrites the same message.
async fn finish(st: &AppState, ctx: &Ctx, scope: &ClientScope, r: &mut Reply) -> ApiResult<()> {
    let video = mark_reviewed(st, scope, &r.video_id).await?;
    if !r.key.is_empty() {
        tag_reviewed(st, &[r.key.as_str()]).await;
    }
    let what = format!("{} set {}", if video.exercise_name.is_empty() { "your" } else { &video.exercise_name }, video.set_index + 1);
    let text = match (r.note.is_empty(), r.key.is_empty()) {
        (false, _) => r.note.clone(),
        (true, false) => format!("Sent you a video reply on {what}."),
        (true, true) => format!("Reviewed {what}."),
    };
    let c = Comment {
        id: r.id.clone(),
        author_id: ctx.user_id().to_string(),
        author_role: Role::Coach,
        author_name: ctx.profile.name.clone(),
        log_id: Some(video.log_id.clone()),
        item_id: Some(video.item_id.clone()),
        set_index: Some(video.set_index),
        text,
        reply_id: (!r.key.is_empty()).then(|| r.id.clone()),
        created_at: r.created_at,
    };
    st.store.put(&scope.pk(), &format!("COMMENT#{}#{}", c.created_at.to_rfc3339(), c.id), &c).await?;
    r.status = VideoStatus::Ready;
    r.upload_id = None;
    r.sent_at.get_or_insert_with(Utc::now);
    st.store.put(&scope.pk(), &reply_sk(&r.id), &*r).await?;
    on_message(st, scope, &c).await;
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyOut {
    #[serde(flatten)]
    reply: Reply,
    url: Option<String>,
}

/// Sent replies, newest first, with playback URLs. The client and their coach.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Vec<ReplyOut>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-replies").await;
    let mut replies: Vec<Reply> = st.store.query_prefix(&scope.pk(), "REPLY#").await?;
    replies.retain(|r| r.status == VideoStatus::Ready);
    replies.sort_by_key(|r| std::cmp::Reverse(r.created_at));
    let out = try_join_all(replies.into_iter().map(|reply| {
        let st = &st;
        async move {
            let url = if reply.key.is_empty() { None } else { Some(presign_get(st, &reply.key).await?) };
            ApiResult::Ok(ReplyOut { reply, url })
        }
    }))
    .await?;
    Ok(Json(out))
}

/// The client has played the reply.
pub async fn seen(State(st): State<AppState>, ctx: Ctx, Path((cid, rid)): Path<(String, String)>) -> ApiResult<Json<Reply>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client marks a reply seen"));
    }
    let mut r = load(&st, &scope, &rid).await?;
    if r.seen_at.is_none() {
        r.seen_at = Some(Utc::now());
        st.store.put(&scope.pk(), &reply_sk(&r.id), &r).await?;
    }
    Ok(Json(r))
}

async fn delete_reply(st: &AppState, scope: &ClientScope, r: &Reply) -> ApiResult<()> {
    if let Some(upload_id) = &r.upload_id {
        let _ = st.s3.abort_multipart_upload().bucket(&st.bucket).key(&r.key).upload_id(upload_id).send().await;
    }
    if !r.key.is_empty() {
        st.s3
            .delete_object()
            .bucket(&st.bucket)
            .key(&r.key)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("s3 delete: {e}")))?;
    }
    st.store.delete(&scope.pk(), &reply_sk(&r.id)).await
}

/// The coach deletes one of their replies (its thread message stays, without the video).
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path((cid, rid)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let scope = coach_scope(&st, &ctx, &cid).await?;
    let r = load(&st, &scope, &rid).await?;
    delete_reply(&st, &scope, &r).await?;
    Ok(Json(json!({ "ok": true })))
}

/// Deleting a set video deletes its replies (routes/videos.rs `remove`).
pub async fn remove_for_video(st: &AppState, scope: &ClientScope, vid: &str) -> ApiResult<()> {
    let replies: Vec<Reply> = st.store.query_prefix(&scope.pk(), "REPLY#").await?;
    for r in replies.iter().filter(|r| r.video_id == vid) {
        delete_reply(st, scope, r).await?;
    }
    Ok(())
}
