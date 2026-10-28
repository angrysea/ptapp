use axum::extract::State;
use axum::Json;
use chrono::{NaiveDate, Utc};
use ptcore::model::{valid_time_zone, Profile, Role, Sex, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{save_profile, Caller, Ctx};
use crate::error::{bad_request, ApiResult};
use crate::AppState;

/// Who am I? `profile` is null until the user accepts an invite or (if
/// listed in ADMIN_EMAILS) sets up an admin account; `adminEligible` tells
/// the onboarding screen whether to offer the latter.
pub async fn get_me(State(st): State<AppState>, caller: Caller) -> ApiResult<Json<Value>> {
    let profile: Option<Profile> = st.store.get(&format!("USER#{}", caller.sub), "PROFILE").await?;
    let admin_eligible = profile.is_none() && st.is_admin_email(&caller);
    // Coaches fill in their profile before using the app (tasks/coaches).
    let coach_profile_complete = match &profile {
        Some(p) if p.role == Role::Coach => Some(super::coaches::complete(&st, &caller.sub).await?),
        _ => None,
    };
    // A client's coach, for the header: their business name, else their name.
    let coach = match &profile {
        Some(p) if p.role == Role::Client => st
            .store
            .get::<Profile>(&format!("USER#{}", p.coach_id), "PROFILE")
            .await?
            .map(|c| json!({ "name": c.name, "businessName": c.business_name })),
        _ => None,
    };
    Ok(Json(json!({
        "userId": caller.sub,
        "email": caller.email,
        "profile": profile,
        "adminEligible": admin_eligible,
        "coachProfileComplete": coach_profile_complete,
        "coach": coach,
    })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateMe {
    name: Option<String>,
    units: Option<UnitSystem>,
    sex: Option<Sex>,
    birth_date: Option<NaiveDate>,
    business_name: Option<String>,
    time_zone: Option<String>,
}

/// `POST /me/consent`: an account made before the data agreement existed
/// agrees to it (the app asks once, on sign-in).
pub async fn consent(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Profile>> {
    let mut p = ctx.profile;
    if p.consented_at.is_none() {
        p.consented_at = Some(Utc::now());
        save_profile(&st, &p).await?;
    }
    Ok(Json(p))
}

pub async fn update_me(State(st): State<AppState>, ctx: Ctx, Json(body): Json<UpdateMe>) -> ApiResult<Json<Profile>> {
    let mut p = ctx.profile;
    if let Some(n) = body.name.filter(|n| !n.trim().is_empty()) {
        p.name = n.trim().to_string();
    }
    if let Some(u) = body.units {
        p.units = u;
    }
    if body.sex.is_some() {
        p.sex = body.sex;
    }
    if body.birth_date.is_some() {
        p.birth_date = body.birth_date;
    }
    if p.role == Role::Coach && body.business_name.is_some() {
        p.business_name = body.business_name;
    }
    if let Some(tz) = body.time_zone {
        if !valid_time_zone(&tz) {
            return Err(bad_request(format!("unknown time zone '{tz}'")));
        }
        p.time_zone = Some(tz);
    }
    save_profile(&st, &p).await?;
    Ok(Json(p))
}
