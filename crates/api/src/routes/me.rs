use axum::extract::State;
use axum::Json;
use chrono::{NaiveDate, Utc};
use ptcore::model::{valid_time_zone, Profile, Role, Sex, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::AppState;

/// Who am I? `profile` is null until the user registers as a coach or
/// accepts an invite.
pub async fn get_me(State(st): State<AppState>, caller: Caller) -> ApiResult<Json<Value>> {
    let profile: Option<Profile> = st.store.get(&format!("USER#{}", caller.sub), "PROFILE").await?;
    Ok(Json(json!({ "userId": caller.sub, "email": caller.email, "profile": profile })))
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

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterCoach {
    name: String,
    business_name: Option<String>,
}

/// Self-service coach signup. Each coach is a tenant (coach_id = own id).
/// Later phases gate this behind admin approval and a Stripe subscription.
pub async fn register_coach(
    State(st): State<AppState>,
    caller: Caller,
    Json(body): Json<RegisterCoach>,
) -> ApiResult<Json<Profile>> {
    if body.name.trim().is_empty() {
        return Err(bad_request("name is required"));
    }
    let profile = Profile {
        user_id: caller.sub.clone(),
        role: Role::Coach,
        coach_id: caller.sub.clone(),
        name: body.name.trim().to_string(),
        email: caller.email,
        units: UnitSystem::Metric,
        sex: None,
        birth_date: None,
        business_name: body.business_name,
        time_zone: None,
        created_at: Utc::now(),
    };
    if !st.store.put_new(&format!("USER#{}", caller.sub), "PROFILE", &profile).await? {
        return Err(ApiError::Conflict(json!({ "message": "account already set up" })));
    }
    Ok(Json(profile))
}
