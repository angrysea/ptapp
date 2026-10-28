//! WB-1: built-in library + coach custom exercises with uploaded demos.

use axum::extract::{Path, State};
use axum::Json;
use ptcore::library::{builtin_exercises, is_builtin};
use ptcore::model::{Exercise, Filming};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{check_size, ext_for, new_id, presign_get, presign_put, s3_object_exists, MAX_VIDEO_BYTES, PENDING_TAG, VIDEO_TYPES, untag};
use crate::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseOut {
    #[serde(flatten)]
    exercise: Exercise,
    demo_video_url: Option<String>,
}

/// Coaches see their own library; clients see their coach's.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<ExerciseOut>>> {
    let mut all = builtin_exercises();
    let custom: Vec<Exercise> = st.store.query_prefix(&format!("COACH#{}", ctx.profile.coach_id), "EXERCISE#").await?;
    all.extend(custom);
    all.sort_by_key(|e| e.name.to_lowercase());
    // `demo_key` is only set once demo_confirm has seen the object, so no
    // per-exercise S3 check is needed here. Presigning is local (no request).
    let mut out = Vec::with_capacity(all.len());
    for e in all {
        let demo_video_url = match &e.demo_key {
            Some(k) => Some(presign_get(&st, k).await?),
            None => None,
        };
        out.push(ExerciseOut { exercise: e, demo_video_url });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExerciseIn {
    name: String,
    #[serde(default)]
    muscle_groups: Vec<String>,
    #[serde(default)]
    equipment: Vec<String>,
    #[serde(default)]
    cues: Vec<String>,
    demo_url: Option<String>,
    /// Filming guidance (VS-5); None clears it.
    #[serde(default)]
    filming: Option<Filming>,
}

fn clean_filming(f: Option<Filming>) -> ApiResult<Option<Filming>> {
    let Some(mut f) = f else { return Ok(None) };
    f.notes = f.notes.trim().to_string();
    f.validate().map_err(bad_request)?;
    Ok(Some(f))
}

fn clean(list: Vec<String>) -> Vec<String> {
    list.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(body): Json<ExerciseIn>) -> ApiResult<Json<Exercise>> {
    let coach = ctx.require_coach()?;
    if body.name.trim().is_empty() {
        return Err(bad_request("name is required"));
    }
    let ex = Exercise {
        id: format!("c-{}", &new_id()[..12]),
        name: body.name.trim().to_string(),
        muscle_groups: clean(body.muscle_groups),
        equipment: clean(body.equipment),
        cues: clean(body.cues),
        demo_url: body.demo_url.filter(|u| u.starts_with("https://")),
        demo_key: None,
        pending_demo_key: None,
        custom: true,
        filming: clean_filming(body.filming)?,
    };
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{}", ex.id), &ex).await?;
    Ok(Json(ex))
}

async fn load_custom(st: &AppState, coach: &str, id: &str) -> ApiResult<Exercise> {
    if is_builtin(id) {
        return Err(ApiError::Forbidden("built-in exercises are read-only; create a custom copy"));
    }
    st.store
        .get(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"))
        .await?
        .ok_or(ApiError::NotFound("exercise"))
}

pub async fn update(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(body): Json<ExerciseIn>,
) -> ApiResult<Json<Exercise>> {
    let coach = ctx.require_coach()?;
    let mut ex = load_custom(&st, coach, &id).await?;
    if !body.name.trim().is_empty() {
        ex.name = body.name.trim().to_string();
    }
    ex.muscle_groups = clean(body.muscle_groups);
    ex.equipment = clean(body.equipment);
    ex.cues = clean(body.cues);
    ex.demo_url = body.demo_url.filter(|u| u.starts_with("https://"));
    ex.filming = clean_filming(body.filming)?;
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"), &ex).await?;
    Ok(Json(ex))
}

/// Best effort: a failure is logged and the object stays (it is not tagged
/// pending, so no lifecycle rule removes it; search the logs to clean up).
async fn delete_object(st: &AppState, key: &str) {
    if let Err(e) = st.s3.delete_object().bucket(&st.bucket).key(key).send().await {
        tracing::warn!("media {key} delete failed: {e}");
    }
}

pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let ex = load_custom(&st, coach, &id).await?;
    st.store.delete(&format!("COACH#{coach}"), &format!("EXERCISE#{id}")).await?;
    for k in ex.demo_key.iter().chain(&ex.pending_demo_key) {
        delete_object(&st, k).await;
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoUpload {
    content_type: String,
    size: i64,
}

/// Step 1 of a demo upload: a presigned PUT URL for the coach's video. The
/// current demo keeps playing until step 2 (`demo_confirm`) swaps it.
pub async fn demo_upload(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(body): Json<DemoUpload>,
) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let mut ex = load_custom(&st, coach, &id).await?;
    let ext = ext_for(&body.content_type, VIDEO_TYPES)?;
    check_size(body.size, MAX_VIDEO_BYTES)?;
    let key = format!("media/{coach}/exercises/{id}/{}.{ext}", &new_id()[..8]);
    let upload = presign_put(&st, &key, &body.content_type, body.size, Some(PENDING_TAG)).await?;
    // An earlier upload that was never confirmed is abandoned.
    if let Some(old) = ex.pending_demo_key.replace(key) {
        delete_object(&st, &old).await;
    }
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"), &ex).await?;
    Ok(Json(json!({ "uploadUrl": upload.url, "uploadHeaders": upload.headers, "contentType": body.content_type })))
}

/// Step 2: once the browser's PUT has succeeded, make the new video the demo
/// and delete the one it replaces.
pub async fn demo_confirm(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Exercise>> {
    let coach = ctx.require_coach()?;
    let mut ex = load_custom(&st, coach, &id).await?;
    let pending = ex.pending_demo_key.clone().ok_or_else(|| bad_request("no demo upload is pending"))?;
    if !s3_object_exists(&st, &pending).await? {
        return Err(bad_request("the video upload has not arrived; try uploading again"));
    }
    // Untag first: a confirmed demo must not be expired by the lifecycle rule.
    untag(&st, &pending).await?;
    ex.pending_demo_key = None;
    let old = ex.demo_key.replace(pending);
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"), &ex).await?;
    if let Some(old) = old {
        delete_object(&st, &old).await;
    }
    Ok(Json(ex))
}
