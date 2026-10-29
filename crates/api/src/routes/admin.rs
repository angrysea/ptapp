//! Admins onboard coaches; there is no open coach signup. An admin invites a
//! coach by email: the invitation is stored with that address and emailed
//! (see mail.rs), the link opens the app's own sign-up page with the email
//! filled in from the invitation, and accepting it creates the coach account
//! (routes/invites.rs). An accepted or expired invitation's link stops
//! working.
//!
//! The first admin bootstraps through ADMIN_EMAILS: a verified email on
//! that list may set up an admin account.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{CoachEntry, Invite, InviteKind, Profile, Role, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::mail::send_invite;
use crate::routes::invites::{issue, new_invite, open_invites, revoke_where, ADMIN_PK};
use crate::AppState;

/// Shown on the join screen of coach invites.
const APP_NAME: &str = "PT Coach";

fn clean_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(bad_request("name must be 1-100 characters"));
    }
    Ok(name.to_string())
}

#[derive(Deserialize)]
pub struct RegisterAdmin {
    name: String,
}

pub async fn register(State(st): State<AppState>, caller: Caller, Json(body): Json<RegisterAdmin>) -> ApiResult<Json<Profile>> {
    if !st.is_admin_email(&caller) {
        return Err(ApiError::Forbidden("this email is not on the admin list"));
    }
    let profile = Profile {
        user_id: caller.sub.clone(),
        role: Role::Admin,
        // Admins belong to no coach's tenant.
        coach_id: String::new(),
        name: clean_name(&body.name)?,
        email: caller.email.trim().to_lowercase(),
        units: UnitSystem::default(),
        sex: None,
        birth_date: None,
        business_name: None,
        time_zone: None,
        // Admins run the service; the agreement is for coaches and clients.
        consented_at: None,
        suspended_at: None,
        former_coach_ids: Vec::new(),
        created_at: Utc::now(),
    };
    if !st.store.put_new(&format!("USER#{}", caller.sub), "PROFILE", &profile).await? {
        return Err(ApiError::Conflict(json!({ "message": "account already set up" })));
    }
    Ok(Json(profile))
}

pub async fn coaches(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<CoachEntry>>> {
    ctx.require_admin()?;
    let mut list: Vec<CoachEntry> = st.store.query_prefix(ADMIN_PK, "COACH#").await?;
    list.sort_by_key(|c| c.name.to_lowercase());
    Ok(Json(list))
}

// ---------------------------------------------------------------- invites

#[derive(Deserialize)]
pub struct CoachInvite {
    email: String,
    name: String,
}

/// The invite plus whether its email went out. A failed send (e.g. the
/// sending domain isn't verified yet, or SES is still in its sandbox) never
/// fails the invite: the admin page then offers the link to share by hand.
pub async fn with_email_status(st: &AppState, invite: &Invite) -> ApiResult<Value> {
    let mut out = serde_json::to_value(invite).map_err(|e| ApiError::Internal(e.to_string()))?;
    let (emailed, error) = match send_invite(st, invite).await {
        Ok(sent) => (sent, None),
        Err(e) => {
            tracing::warn!("invite email to {} failed: {e}", invite.email);
            (false, Some(e))
        }
    };
    out["emailed"] = json!(emailed);
    out["emailError"] = json!(error);
    Ok(out)
}

pub async fn create_invite(State(st): State<AppState>, ctx: Ctx, Json(body): Json<CoachInvite>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let invite = new_invite(InviteKind::Coach, "", APP_NAME, &body.email, &clean_name(&body.name)?)?;
    issue(&st, &invite).await?;
    Ok(Json(with_email_status(&st, &invite).await?))
}

/// Sends a pending coach invitation's email again.
pub async fn resend_invite(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let code = code.to_uppercase();
    let invite = open_invites(&st, ADMIN_PK)
        .await?
        .into_iter()
        .find(|i| i.code == code)
        .ok_or(ApiError::NotFound("invite"))?;
    Ok(Json(with_email_status(&st, &invite).await?))
}

pub async fn list_invites(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Invite>>> {
    ctx.require_admin()?;
    Ok(Json(open_invites(&st, ADMIN_PK).await?))
}

pub async fn revoke_invite(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    revoke_where(&st, &code, |i| i.kind == InviteKind::Coach).await?;
    Ok(Json(json!({ "ok": true })))
}
