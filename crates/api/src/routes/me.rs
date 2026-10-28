use axum::extract::State;
use axum::Json;
use chrono::NaiveDate;
use ptcore::model::{valid_time_zone, Profile, Role, Sex, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
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
    Ok(Json(json!({
        "userId": caller.sub,
        "email": caller.email,
        "profile": profile,
        "adminEligible": admin_eligible,
        "coachProfileComplete": coach_profile_complete,
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
    st.store.put(&format!("USER#{}", p.user_id), "PROFILE", &p).await?;
    Ok(Json(p))
}
