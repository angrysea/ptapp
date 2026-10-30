//! VS-1..VS-3: a client's video of a set, uploaded in resumable parts.
//!
//! The browser trims and re-encodes the clip (or, when it can't, uploads the
//! original with trim points), then:
//!   1. `POST /videos` creates the record and an S3 multipart upload tagged
//!      pending, and returns one URL per part, each signed for its exact size;
//!   2. uploads the parts, keeping their ETags; `POST …/parts` re-issues URLs
//!      that expired while it was offline;
//!   3. `POST …/complete` assembles the parts, checks the size, removes the
//!      pending tag and marks the video ready.
//!
//! Abandoned uploads are cleaned up by the media bucket's lifecycle rules
//! (incomplete multipart uploads after 2 days; pending-tagged objects).
//!
//! VS-4: completing an upload also puts the video in the coach's review
//! queue, oldest first; marking it reviewed takes it out.
//!
//! Retention: marking a video reviewed tags its objects `reviewed=true`, and
//! the media bucket's lifecycle rule deletes tagged objects
//! `video_retention_days` after upload (S3 counts from creation). Records past
//! that are hidden here, so no one gets a dead link. Unreviewed videos are
//! never deleted automatically.

use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart, Tag, Tagging};
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use futures::future::try_join_all;
use ptcore::model::{ReviewEntry, Video, VideoStatus};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use ptcore::credits::Service;
use ptcore::team::Perm;
use crate::auth::{client_scope, client_scope_as, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{
    audit, audit_client, check_size, dated_id, ext_for, presign_get, presign_part, presign_put, s3_object_exists, s3_object_size,
    untag, Upload, PENDING_TAG, VIDEO_TYPES,
};
use crate::AppState;

/// S3 needs every part but the last to be at least 5 MiB.
pub const PART_SIZE: i64 = 8 * 1024 * 1024;
/// Re-encoded clips are far smaller; this covers originals on the fallback path.
pub const MAX_SET_VIDEO_BYTES: i64 = 500 * 1024 * 1024;
const MAX_POSTER_BYTES: i64 = 2 * 1024 * 1024;
/// BRD VS-2. A little over 60 s allows for encoder rounding.
const MAX_SECONDS: f64 = 60.5;

/// Sizes of the parts a `size`-byte upload is split into, in order.
pub fn part_sizes(size: i64, part_size: i64) -> Vec<i64> {
    let mut parts = Vec::new();
    let mut left = size;
    while left > 0 {
        parts.push(left.min(part_size));
        left -= part_size;
    }
    parts
}

fn video_sk(id: &str) -> String {
    format!("VIDEO#{id}")
}

/// The video's place in its coach's review queue. Derived from the video, so
/// marking it reviewed can find the entry without a lookup; fixed-width UTC
/// timestamps sort oldest first.
fn review_sk(v: &Video) -> String {
    format!("REVIEW#{}#{}", v.created_at.to_rfc3339_opts(SecondsFormat::Millis, true), v.id)
}

fn poster_key_for(video_key: &str) -> String {
    match video_key.rsplit_once('.') {
        Some((stem, _)) => format!("{stem}.jpg"),
        None => format!("{video_key}.jpg"),
    }
}

/// Ids here come from the device; keep them to the same shape as log ids.
fn check_id(what: &str, id: &str) -> ApiResult<()> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(bad_request(format!("{what} must be 1-64 characters of [A-Za-z0-9-]")));
    }
    Ok(())
}

/// Uploading is the client's own act; coaches only watch.
async fn client_owned(st: &AppState, ctx: &Ctx, cid: &str) -> ApiResult<ClientScope> {
    let scope = client_scope(st, ctx, cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client uploads their videos"));
    }
    Ok(scope)
}

/// Whether the lifecycle rule has (or soon will have) deleted a reviewed video.
pub fn expired(v: &Video, now: DateTime<Utc>, retention_days: i64) -> bool {
    v.reviewed_at.is_some() && v.created_at + Duration::days(retention_days) <= now
}

async fn load(st: &AppState, scope: &ClientScope, vid: &str) -> ApiResult<Video> {
    st.store.get(&scope.pk(), &video_sk(vid)).await?.ok_or(ApiError::NotFound("video"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartUrl {
    n: i32,
    size: i64,
    url: String,
    headers: std::collections::BTreeMap<String, String>,
}

/// URLs for parts `numbers` of a `size`-byte multipart upload, each signed
/// for its exact size. Shared with coach replies (routes/replies.rs).
pub async fn sign_parts(st: &AppState, key: &str, upload_id: Option<&str>, size: i64, part_size: i64, numbers: &[i32]) -> ApiResult<Vec<PartUrl>> {
    let upload_id = upload_id.ok_or_else(|| bad_request("this upload is already finished"))?;
    let sizes = part_sizes(size, part_size);
    // Each URL costs a signature: never sign more than the upload has parts.
    if numbers.len() > sizes.len() {
        return Err(bad_request("more parts requested than the upload has"));
    }
    let mut out = Vec::with_capacity(numbers.len());
    for &n in numbers {
        let size = *sizes.get((n as usize).wrapping_sub(1)).ok_or_else(|| bad_request(format!("no part {n}")))?;
        let Upload { url, headers } = presign_part(st, key, upload_id, n, size).await?;
        out.push(PartUrl { n, size, url, headers });
    }
    Ok(out)
}

async fn part_urls(st: &AppState, v: &Video, numbers: &[i32]) -> ApiResult<Vec<PartUrl>> {
    sign_parts(st, &v.key, v.upload_id.as_deref(), v.size_bytes, v.part_size, numbers).await
}

/// Starts a multipart upload tagged pending (the lifecycle rules clean up
/// ones never finished) and returns its id.
pub async fn start_multipart(st: &AppState, key: &str, content_type: &str) -> ApiResult<String> {
    let mp = st
        .s3
        .create_multipart_upload()
        .bucket(&st.bucket)
        .key(key)
        .content_type(content_type)
        .tagging(PENDING_TAG)
        .send()
        .await
        .map_err(|e| ApiError::Internal(format!("s3 create multipart: {e}")))?;
    Ok(mp.upload_id().ok_or_else(|| ApiError::Internal("s3 returned no upload id".into()))?.to_string())
}

/// Assembles uploaded parts into the object, checks its size and removes the
/// pending tag. Safe to repeat after a lost response.
pub async fn assemble(st: &AppState, key: &str, upload_id: &str, size: i64, part_size: i64, parts: &[DonePart]) -> ApiResult<()> {
    let expected = part_sizes(size, part_size).len();
    let mut done: Vec<&DonePart> = parts.iter().collect();
    done.sort_by_key(|p| p.n);
    if done.len() != expected || done.iter().enumerate().any(|(i, p)| p.n as usize != i + 1) {
        return Err(bad_request(format!("send all {expected} parts, numbered 1 to {expected}")));
    }
    let parts = done.iter().map(|p| CompletedPart::builder().part_number(p.n).e_tag(&p.etag).build()).collect();
    let res = st
        .s3
        .complete_multipart_upload()
        .bucket(&st.bucket)
        .key(key)
        .upload_id(upload_id)
        .multipart_upload(CompletedMultipartUpload::builder().set_parts(Some(parts)).build())
        .send()
        .await;
    if let Err(e) = res {
        // A retry after a lost response finds the upload already assembled.
        if !s3_object_exists(st, key).await? {
            return Err(bad_request(format!("the upload is incomplete; upload the missing parts and try again ({e})")));
        }
    }
    // Part sizes were signed into the URLs, so this only fails if parts were
    // mixed up; such an object is useless, so it goes.
    let got = s3_object_size(st, key).await?.unwrap_or(0);
    if got != size {
        let _ = st.s3.delete_object().bucket(&st.bucket).key(key).send().await;
        return Err(bad_request(format!("uploaded {got} bytes but expected {size}; please upload it again")));
    }
    untag(st, key).await
}

/// Starts the retention clock on objects: the lifecycle rule deletes objects
/// tagged `reviewed=true`. A failed tag only means the object is kept.
pub async fn tag_reviewed(st: &AppState, keys: &[&str]) {
    let Ok(tag) = Tag::builder().key("reviewed").value("true").build() else { return };
    let Ok(tagging) = Tagging::builder().tag_set(tag).build() else { return };
    for key in keys {
        if let Err(e) = st.s3.put_object_tagging().bucket(&st.bucket).key(*key).tagging(tagging.clone()).send().await {
            tracing::warn!("tagging {key} reviewed failed: {e}");
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVideo {
    log_id: String,
    item_id: String,
    set_index: u32,
    exercise_id: String,
    #[serde(default)]
    exercise_name: String,
    content_type: String,
    size: i64,
    duration_sec: Option<f64>,
    trim_start: Option<f64>,
    trim_end: Option<f64>,
    /// Size of the poster JPEG, when the device made one.
    poster_size: Option<i64>,
}

pub async fn create(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<NewVideo>,
) -> ApiResult<Json<Value>> {
    let scope = client_owned(&st, &ctx, &cid).await?;
    // Each video awaiting review holds a review credit (routes/credits.rs).
    super::credits::require(&st, &scope, Service::VideoReview).await?;
    check_id("logId", &body.log_id)?;
    check_id("itemId", &body.item_id)?;
    let ext = ext_for(&body.content_type, VIDEO_TYPES)?;
    check_size(body.size, MAX_SET_VIDEO_BYTES)?;
    if let Some(p) = body.poster_size {
        check_size(p, MAX_POSTER_BYTES)?;
    }
    // The cap applies to what gets watched: the trimmed range if there is one.
    let watched = match (body.trim_start, body.trim_end) {
        (Some(a), Some(b)) if a >= 0.0 && b > a => Some(b - a),
        (None, None) => body.duration_sec,
        _ => return Err(bad_request("trimStart and trimEnd go together, with trimEnd after trimStart")),
    };
    if watched.is_some_and(|s| !(0.0..=MAX_SECONDS).contains(&s)) {
        return Err(bad_request("videos can be at most 60 seconds"));
    }

    let id = dated_id(scope.today());
    let key = format!("media/{}/{}/videos/{id}.{ext}", scope.client.coach_id, scope.client.user_id);
    let upload_id = start_multipart(&st, &key, &body.content_type).await?;

    let video = Video {
        id,
        log_id: body.log_id,
        item_id: body.item_id,
        set_index: body.set_index,
        exercise_id: body.exercise_id,
        exercise_name: body.exercise_name,
        status: VideoStatus::Uploading,
        key,
        content_type: body.content_type,
        size_bytes: body.size,
        upload_id: Some(upload_id),
        part_size: PART_SIZE,
        poster_key: None,
        duration_sec: body.duration_sec,
        trim_start: body.trim_start,
        trim_end: body.trim_end,
        created_at: Utc::now(),
        reviewed_at: None,
    };
    let numbers: Vec<i32> = (1..=part_sizes(video.size_bytes, video.part_size).len() as i32).collect();
    let parts = part_urls(&st, &video, &numbers).await?;
    let poster = match body.poster_size {
        Some(size) => {
            let Upload { url, headers } = presign_put(&st, &poster_key_for(&video.key), "image/jpeg", size, Some(PENDING_TAG)).await?;
            Some(json!({ "url": url, "headers": headers }))
        }
        None => None,
    };
    st.store.put(&scope.pk(), &video_sk(&video.id), &video).await?;
    // The upload now holds a review credit; if another upload took the last
    // one at the same moment, this one is undone.
    if let Err(e) = super::credits::confirm_hold(&st, &scope, Service::VideoReview).await {
        st.store.delete(&scope.pk(), &video_sk(&video.id)).await?;
        if let Some(upload_id) = &video.upload_id {
            let _ = st.s3.abort_multipart_upload().bucket(&st.bucket).key(&video.key).upload_id(upload_id).send().await;
        }
        return Err(e);
    }
    Ok(Json(json!({ "video": video, "parts": parts, "poster": poster })))
}

#[derive(Deserialize)]
pub struct PartsQuery {
    pub parts: Vec<i32>,
}

/// Fresh URLs for parts whose URLs expired, e.g. after a stretch offline.
pub async fn parts(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, vid)): Path<(String, String)>,
    Json(body): Json<PartsQuery>,
) -> ApiResult<Json<Value>> {
    let scope = client_owned(&st, &ctx, &cid).await?;
    let v = load(&st, &scope, &vid).await?;
    let mut numbers = body.parts;
    numbers.sort_unstable();
    numbers.dedup();
    Ok(Json(json!({ "parts": part_urls(&st, &v, &numbers).await? })))
}

#[derive(Deserialize)]
pub struct DonePart {
    pub n: i32,
    pub etag: String,
}

#[derive(Deserialize)]
pub struct Complete {
    pub parts: Vec<DonePart>,
}

/// Assembles the uploaded parts and marks the video ready. Safe to repeat:
/// a device that lost the first response just gets the ready video again.
pub async fn complete(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, vid)): Path<(String, String)>,
    Json(body): Json<Complete>,
) -> ApiResult<Json<Video>> {
    let scope = client_owned(&st, &ctx, &cid).await?;
    let mut v = load(&st, &scope, &vid).await?;
    if v.status == VideoStatus::Ready {
        return Ok(Json(v));
    }
    let upload_id = v.upload_id.clone().ok_or_else(|| ApiError::Internal("uploading video without an upload id".into()))?;
    assemble(&st, &v.key, &upload_id, v.size_bytes, v.part_size, &body.parts).await?;
    let poster = poster_key_for(&v.key);
    if s3_object_exists(&st, &poster).await? {
        untag(&st, &poster).await?;
        v.poster_key = Some(poster);
    }
    v.status = VideoStatus::Ready;
    v.upload_id = None;
    // Queue first: if marking the video ready then fails, a retried complete
    // writes the same entry again rather than leaving a ready video unqueued.
    let entry = ReviewEntry {
        video_id: v.id.clone(),
        client_id: scope.client.user_id.clone(),
        client_name: scope.client.name.clone(),
        log_id: v.log_id.clone(),
        item_id: v.item_id.clone(),
        set_index: v.set_index,
        exercise_name: v.exercise_name.clone(),
        key: v.key.clone(),
        poster_key: v.poster_key.clone(),
        duration_sec: v.duration_sec,
        trim_start: v.trim_start,
        trim_end: v.trim_end,
        created_at: v.created_at,
    };
    st.store.put(&format!("COACH#{}", scope.client.coach_id), &review_sk(&v), &entry).await?;
    st.store.put(&scope.pk(), &video_sk(&v.id), &v).await?;
    Ok(Json(v))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewOut {
    #[serde(flatten)]
    entry: ReviewEntry,
    url: String,
    poster_url: Option<String>,
}

/// The coach's unreviewed videos across all their clients, oldest first.
pub async fn queue(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<ReviewOut>>> {
    let coach = ctx.require(Perm::Review)?;
    let mut entries: Vec<ReviewEntry> = st.store.query_prefix(&format!("COACH#{coach}"), "REVIEW#").await?;
    entries.retain(|e| ctx.sees_client(&e.client_id));
    let mut clients: Vec<&str> = entries.iter().map(|e| e.client_id.as_str()).collect();
    clients.sort_unstable();
    clients.dedup();
    for c in clients {
        audit_client(&st, &ctx, &format!("CLIENT#{c}"), "view-video-queue").await;
    }
    let out = try_join_all(entries.into_iter().map(|entry| {
        let st = &st;
        async move {
            let url = presign_get(st, &entry.key).await?;
            let poster_url = match &entry.poster_key {
                Some(k) => Some(presign_get(st, k).await?),
                None => None,
            };
            ApiResult::Ok(ReviewOut { entry, url, poster_url })
        }
    }))
    .await?;
    Ok(Json(out))
}

/// The client's coach marks a video reviewed: it leaves the queue and shows
/// as reviewed in the client's logs. Repeating it is harmless.
pub async fn reviewed(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, vid)): Path<(String, String)>,
) -> ApiResult<Json<Video>> {
    ctx.require(Perm::Review)?;
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Review]).await?;
    Ok(Json(mark_reviewed(&st, &scope, &vid).await?))
}

/// Marks a set video reviewed: out of the coach's queue, retention started.
/// Also done by sending a reply (routes/replies.rs). Repeating it is harmless.
pub async fn mark_reviewed(st: &AppState, scope: &ClientScope, vid: &str) -> ApiResult<Video> {
    let mut v = load(st, scope, vid).await?;
    if v.status != VideoStatus::Ready {
        return Err(bad_request("this video hasn't finished uploading"));
    }
    if v.reviewed_at.is_none() {
        v.reviewed_at = Some(Utc::now());
        st.store.put(&scope.pk(), &video_sk(&v.id), &v).await?;
        // The review is delivered: the credit the upload held is used.
        super::credits::spend(st, scope, Service::VideoReview, &format!("Video reviewed: {}", v.exercise_name), false).await?;
    }
    st.store.delete(&format!("COACH#{}", scope.client.coach_id), &review_sk(&v)).await?;
    let keys: Vec<&str> = std::iter::once(v.key.as_str()).chain(v.poster_key.as_deref()).collect();
    tag_reviewed(st, &keys).await;
    Ok(v)
}

/// A set video, for checks by other routes (replies).
pub async fn find(st: &AppState, scope: &ClientScope, vid: &str) -> ApiResult<Video> {
    load(st, scope, vid).await
}

/// VS-3: the client deletes their own video: an unfinished upload is
/// aborted, the objects and the coach's queue entry go with it.
pub async fn remove(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, vid)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    let scope = client_owned(&st, &ctx, &cid).await?;
    let v = load(&st, &scope, &vid).await?;
    if let Some(upload_id) = &v.upload_id {
        // Already gone (e.g. expired by the lifecycle rule) is fine.
        let _ = st.s3.abort_multipart_upload().bucket(&st.bucket).key(&v.key).upload_id(upload_id).send().await;
    }
    // The poster may exist even if it never got recorded (an upload that was
    // never completed), so delete it by its derived key.
    for key in [v.key.clone(), poster_key_for(&v.key)] {
        st.s3
            .delete_object()
            .bucket(&st.bucket)
            .key(&key)
            .send()
            .await
            .map_err(|e| ApiError::Internal(format!("s3 delete: {e}")))?;
    }
    st.store.delete(&format!("COACH#{}", scope.client.coach_id), &review_sk(&v)).await?;
    super::replies::remove_for_video(&st, &scope, &v.id).await?;
    st.store.delete(&scope.pk(), &video_sk(&v.id)).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListQuery {
    log_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoOut {
    #[serde(flatten)]
    video: Video,
    url: String,
    poster_url: Option<String>,
}

/// Ready videos, newest first, with short-lived playback URLs. The client
/// and their coach can list them.
pub async fn list(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<VideoOut>>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Review, Perm::Progress, Perm::Messaging]).await?;
    audit(&st, &ctx, &scope, "view-videos").await;
    let mut videos: Vec<Video> = st.store.query_prefix(&scope.pk(), "VIDEO#").await?;
    let now = Utc::now();
    videos.retain(|v| {
        v.status == VideoStatus::Ready && !expired(v, now, st.video_retention_days) && q.log_id.as_ref().is_none_or(|l| *l == v.log_id)
    });
    videos.sort_by_key(|v| std::cmp::Reverse(v.created_at));
    let out = try_join_all(videos.into_iter().map(|video| {
        let st = &st;
        async move {
            let url = presign_get(st, &video.key).await?;
            let poster_url = match &video.poster_key {
                Some(k) => Some(presign_get(st, k).await?),
                None => None,
            };
            ApiResult::Ok(VideoOut { video, url, poster_url })
        }
    }))
    .await?;
    Ok(Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_into_parts() {
        assert_eq!(part_sizes(1, PART_SIZE), vec![1]);
        assert_eq!(part_sizes(PART_SIZE, PART_SIZE), vec![PART_SIZE]);
        assert_eq!(part_sizes(PART_SIZE + 5, PART_SIZE), vec![PART_SIZE, 5]);
        let big = part_sizes(MAX_SET_VIDEO_BYTES, PART_SIZE);
        assert_eq!(big.len(), 63);
        assert_eq!(big.iter().sum::<i64>(), MAX_SET_VIDEO_BYTES);
        assert!(big[..big.len() - 1].iter().all(|&p| p == PART_SIZE));
        assert!(part_sizes(0, PART_SIZE).is_empty());
    }

    #[test]
    fn reviewed_videos_expire_after_the_retention_period() {
        let created = Utc::now() - Duration::days(200);
        let mut v = Video {
            id: "v".into(),
            log_id: "l".into(),
            item_id: "i".into(),
            set_index: 0,
            exercise_id: "e".into(),
            exercise_name: "E".into(),
            status: VideoStatus::Ready,
            key: "k.mp4".into(),
            content_type: "video/mp4".into(),
            size_bytes: 1,
            upload_id: None,
            part_size: PART_SIZE,
            poster_key: None,
            duration_sec: None,
            trim_start: None,
            trim_end: None,
            created_at: created,
            reviewed_at: None,
        };
        assert!(!expired(&v, Utc::now(), 180), "unreviewed videos are kept");
        v.reviewed_at = Some(created + Duration::days(1));
        assert!(expired(&v, Utc::now(), 180));
        assert!(!expired(&v, Utc::now(), 365));
    }

    #[test]
    fn poster_sits_next_to_the_video() {
        assert_eq!(poster_key_for("media/c/k/videos/2026-10-27-ab.mp4"), "media/c/k/videos/2026-10-27-ab.jpg");
    }
}
