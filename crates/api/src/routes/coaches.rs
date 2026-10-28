//! Coach profiles and the public coach directory (tasks/coaches/plan.md).
//! The full profile (`USER#{coach}` / `COACHPROFILE`) is read only by the
//! coach and admins. Saving a complete, listed profile writes its public copy
//! to `DIRECTORY` / `COACH#{coach}`, and the public routes read nothing else,
//! so private fields can't reach them.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::coach::{CoachProfile, PublicCoach};
use ptcore::model::{CoachEntry, Profile, Role};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::invites::ADMIN_PK;
use crate::util::{check_size, ext_for, new_id, presign_get, presign_put, IMAGE_TYPES, MAX_PHOTO_BYTES};
use crate::AppState;

const SK: &str = "COACHPROFILE";
const DIRECTORY: &str = "DIRECTORY";

fn user_pk(coach: &str) -> String {
    format!("USER#{coach}")
}

/// The coach's profile, if they've saved one.
pub async fn load(st: &AppState, coach: &str) -> ApiResult<Option<CoachProfile>> {
    st.store.get(&user_pk(coach), SK).await
}

/// Whether a coach has a complete profile (`GET /me` reports it).
pub async fn complete(st: &AppState, coach: &str) -> ApiResult<bool> {
    Ok(load(st, coach).await?.is_some_and(|p| p.problems().is_empty()))
}

/// Writes the profile and brings the directory entry in line with it.
async fn save(st: &AppState, coach: &str, profile: &CoachProfile) -> ApiResult<()> {
    st.store.put(&user_pk(coach), SK, profile).await?;
    let entry = format!("COACH#{coach}");
    if profile.shows_in_directory() {
        st.store.put(DIRECTORY, &entry, &profile.public(coach)).await
    } else {
        st.store.delete(DIRECTORY, &entry).await
    }
}

/// A photo URL for public pages (the media bucket is private).
async fn with_photo(st: &AppState, c: &PublicCoach) -> ApiResult<Value> {
    let mut v = serde_json::to_value(c).map_err(|e| ApiError::Internal(e.to_string()))?;
    v["photoUrl"] = match &c.photo_key {
        Some(k) => json!(presign_get(st, k).await?),
        None => Value::Null,
    };
    Ok(v)
}

// ---------------------------------------------------------------- the coach's own

pub async fn get_mine(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let profile = load(&st, coach).await?;
    let photo_url = match profile.as_ref().and_then(|p| p.photo_key.as_deref()) {
        Some(k) => Some(presign_get(&st, k).await?),
        None => None,
    };
    let problems = profile.as_ref().map(CoachProfile::problems);
    Ok(Json(json!({ "profile": profile, "photoUrl": photo_url, "problems": problems })))
}

pub async fn put_mine(State(st): State<AppState>, ctx: Ctx, Json(mut body): Json<CoachProfile>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?.to_string();
    body.normalize();
    let problems = body.problems();
    if !problems.is_empty() {
        return Err(bad_request(problems.join("; ")));
    }
    if let Some(k) = &body.photo_key
        && !k.starts_with(&format!("media/{coach}/profile/"))
    {
        return Err(bad_request("that photo isn't yours"));
    }
    // Only an admin hides a coach; keep whatever they set.
    body.hidden_by_admin = load(&st, &coach).await?.is_some_and(|p| p.hidden_by_admin);
    body.updated_at = Utc::now();
    save(&st, &coach, &body).await?;

    // Clients, invitations and the admin's list show the display name.
    if ctx.profile.name != body.display_name {
        let mut p: Profile = ctx.profile.clone();
        p.name = body.display_name.clone();
        st.store.put(&user_pk(&coach), "PROFILE", &p).await?;
        let entry_sk = format!("COACH#{coach}");
        if let Some(mut entry) = st.store.get::<CoachEntry>(ADMIN_PK, &entry_sk).await? {
            entry.name = body.display_name.clone();
            st.store.put(ADMIN_PK, &entry_sk, &entry).await?;
        }
    }
    Ok(Json(json!({ "profile": body, "listed": body.shows_in_directory() })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhotoIn {
    content_type: String,
    size: i64,
}

/// A presigned upload for a new profile photo; the key is saved with the profile.
pub async fn photo_upload(State(st): State<AppState>, ctx: Ctx, Json(body): Json<PhotoIn>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let ext = ext_for(&body.content_type, IMAGE_TYPES)?;
    check_size(body.size, MAX_PHOTO_BYTES)?;
    let key = format!("media/{coach}/profile/{}.{ext}", new_id());
    let up = presign_put(&st, &key, &body.content_type, body.size, None).await?;
    Ok(Json(json!({ "photoKey": key, "uploadUrl": up.url, "uploadHeaders": up.headers })))
}

// ---------------------------------------------------------------- public (no sign-in)

pub async fn public_list(State(st): State<AppState>) -> ApiResult<Json<Vec<Value>>> {
    let mut coaches: Vec<PublicCoach> = st.store.query_prefix(DIRECTORY, "COACH#").await?;
    coaches.sort_by_key(|c| (!c.accepting_clients, c.display_name.to_lowercase()));
    let mut out = Vec::with_capacity(coaches.len());
    for c in &coaches {
        out.push(with_photo(&st, c).await?);
    }
    Ok(Json(out))
}

pub async fn public_one(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let c: PublicCoach = st.store.get(DIRECTORY, &format!("COACH#{id}")).await?.ok_or(ApiError::NotFound("coach not found"))?;
    Ok(Json(with_photo(&st, &c).await?))
}

// ---------------------------------------------------------------- admin

/// A coach's full profile, private details included.
pub async fn admin_get(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let account: Profile = st.store.get(&user_pk(&id), "PROFILE").await?.filter(|p: &Profile| p.role == Role::Coach).ok_or(ApiError::NotFound("coach not found"))?;
    let profile = load(&st, &id).await?;
    let photo_url = match profile.as_ref().and_then(|p| p.photo_key.as_deref()) {
        Some(k) => Some(presign_get(&st, k).await?),
        None => None,
    };
    let listed = profile.as_ref().is_some_and(CoachProfile::shows_in_directory);
    Ok(Json(json!({ "account": account, "profile": profile, "photoUrl": photo_url, "listed": listed })))
}

#[derive(Deserialize)]
pub struct Hide {
    hidden: bool,
}

/// Hides a coach from the directory, or lets them back in.
pub async fn admin_hide(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<Hide>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let mut profile = load(&st, &id).await?.ok_or(ApiError::NotFound("coach not found"))?;
    profile.hidden_by_admin = body.hidden;
    save(&st, &id, &profile).await?;
    Ok(Json(json!({ "hidden": profile.hidden_by_admin, "listed": profile.shows_in_directory() })))
}
