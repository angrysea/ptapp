//! WB-1: built-in library + coach custom exercises with uploaded demos.

use axum::extract::{Path, State};
use axum::Json;
use ptcore::library::{builtin_exercises, is_builtin};
use ptcore::model::Exercise;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{ext_for, new_id, presign_get, presign_put, s3_object_exists, VIDEO_TYPES};
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
    let mut out = Vec::with_capacity(all.len());
    for e in all {
        let mut demo_key = e.demo_key.clone();
        if let Some(k) = &demo_key {
            let exists = s3_object_exists(&st, k).await?;
            if !exists {
                // Upload never completed; clear the stale key from the record.
                demo_key = None;
                if e.custom {
                    let mut cleared = e.clone();
                    cleared.demo_key = None;
                    st.store.put(&format!("COACH#{}", ctx.profile.coach_id), &format!("EXERCISE#{}", cleared.id), &cleared).await?;
                }
            }
        }
        let demo_video_url = match &demo_key {
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
        custom: true,
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
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"), &ex).await?;
    Ok(Json(ex))
}

pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let ex = load_custom(&st, coach, &id).await?;
    st.store.delete(&format!("COACH#{coach}"), &format!("EXERCISE#{id}")).await?;
    if let Some(k) = ex.demo_key {
        if let Err(e) = st.s3.delete_object().bucket(&st.bucket).key(&k).send().await {
            tracing::warn!("demo video {k} delete failed: {e}"); // bucket lifecycle will reclaim it
        }
    }
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DemoUpload {
    content_type: String,
}

/// Returns a presigned PUT URL for the coach's own demo video.
pub async fn demo_upload(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(body): Json<DemoUpload>,
) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let mut ex = load_custom(&st, coach, &id).await?;
    let ext = ext_for(&body.content_type, VIDEO_TYPES)?;
    let key = format!("media/{coach}/exercises/{id}/{}.{ext}", &new_id()[..8]);
    let upload_url = presign_put(&st, &key, &body.content_type).await?;
    if let Some(old) = ex.demo_key.replace(key) {
        if let Err(e) = st.s3.delete_object().bucket(&st.bucket).key(&old).send().await {
            tracing::warn!("old demo video {old} delete failed: {e}");
        }
    }
    st.store.put(&format!("COACH#{coach}"), &format!("EXERCISE#{id}"), &ex).await?;
    Ok(Json(json!({ "uploadUrl": upload_url, "contentType": body.content_type })))
}
