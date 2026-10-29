//! Coach request queue: the landing page lets a coach ask to join without
//! a link (docs/ARCHITECTURE.md); signup itself stays invitation-only —
//! approving a request creates and emails exactly the same single-use
//! invitation an admin makes by hand (routes/admin.rs, routes/invites.rs).
//! The anonymous POST is the only door in: it carries an email and a name,
//! holds one record per email (the sk is the email), and its records expire
//! through DynamoDB `ttl` after 30 days, so a spammer can age the queue
//! out; a public rate-limiter belongs once the club outgrows this MVP.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{Invite, InviteKind};
use ptcore::requests::{Request, Settings, Status, MAX_EMAIL, MAX_NAME};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::admin::{APP_NAME, with_email_status};
use crate::routes::invites::{ADMIN_PK, issue, new_invite};
use crate::AppState;

/// A pending request sits in the queue this long if nothing happens.
const REQUEST_TTL_DAYS: i64 = 30;

fn email_sk(email: &str) -> String {
    format!("REQUEST#{email}")
}

/// Fresh expiry on every transition, like an invite's ttl.
fn ttl() -> i64 {
    Utc::now().timestamp() + REQUEST_TTL_DAYS * 24 * 3600
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestIn {
    email: String,
    #[serde(default)]
    name: String,
}

/// Approves one record: create and email the invitation, mark the queue
/// entry. Used by the admin's approve route and by auto-approve.
async fn approve(st: &AppState, req: Request) -> ApiResult<(Request, Value)> {
    // The name was typed by an anonymous visitor: it stays in the queue for
    // the admin but never goes into an email we send to someone else.
    let inv: Invite = new_invite(InviteKind::Coach, "", APP_NAME, &req.email, "")?;
    issue(st, &inv).await?;
    let out = with_email_status(st, &inv).await?;
    let mut r = req;
    r.invited(inv.code);
    r.updated_at = Utc::now();
    r.ttl = ttl();
    st.store.put(ADMIN_PK, &email_sk(&r.email), &r).await?;
    Ok((r, out))
}

/// `POST /public/coach-requests` (no sign-in): the landing page's form.
/// Answers only about the email the visitor just typed; the queue itself
/// reads stay behind `require_admin`.
pub async fn request(State(st): State<AppState>, Json(body): Json<RequestIn>) -> ApiResult<Json<Value>> {
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') || email.chars().count() > MAX_EMAIL {
        return Err(bad_request("a valid email is required (up to 80 characters)"));
    }
    if body.name.trim().chars().count() > MAX_NAME {
        return Err(bad_request("a name is at most 100 characters"));
    }
    let found: Option<Request> = st.store.get(ADMIN_PK, &email_sk(&email)).await?;
    // An invitation already went to this email: say so rather than queue or
    // send another.
    if found.as_ref().is_some_and(|r| r.status == Status::Invited) {
        return Ok(Json(json!({ "email": email, "status": "invited" })));
    }
    let now = Utc::now();
    let had_one = found.is_some();
    let fresh = Request {
        email: email.clone(),
        name: body.name.trim().to_string(),
        status: Status::Pending,
        requested_at: now,
        updated_at: now,
        invite_code: None,
        ttl: ttl(),
    };
    let mut r = found.unwrap_or(fresh);
    if had_one && r.status == Status::Pending {
        return Ok(Json(json!({ "email": email, "status": r.status })));
    }
    // A denied email asking again goes back on the queue with any new name.
    r.re_request();
    r.name = body.name.trim().to_string();
    r.ttl = ttl();
    let settings: Option<Settings> = st.store.get(ADMIN_PK, "SETTINGS#AUTO").await?;
    if let Some(_auto) = settings.filter(|s| s.auto_approve) {
        let (invited, out) = approve(&st, r).await?;
        // Don't hand the invitation's code back to an anonymous visitor:
        // they get the email instead, like every other invitation.
        return Ok(Json(json!({ "email": email, "status": invited.status, "emailed": out["emailed"] })));
    }
    st.store.put(ADMIN_PK, &email_sk(&email), &r).await?;
    Ok(Json(json!({ "email": email, "status": r.status })))
}

/// `GET /admin/coach-requests`: newest first, with each one's status.
pub async fn queue(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Request>>> {
    ctx.require_admin()?;
    let mut out: Vec<Request> = st.store.query_prefix(ADMIN_PK, "REQUEST#").await?;
    out.sort_by_key(|r| r.requested_at);
    out.reverse();
    Ok(Json(out))
}

pub async fn get_settings(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Settings>> {
    ctx.require_admin()?;
    let s: Option<Settings> = st.store.get(ADMIN_PK, "SETTINGS#AUTO").await?;
    Ok(Json(s.unwrap_or(Settings { auto_approve: false, updated_at: Utc::now() })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIn {
    auto_approve: bool,
}

pub async fn put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<SettingsIn>) -> ApiResult<Json<Settings>> {
    ctx.require_admin()?;
    let s = Settings { auto_approve: body.auto_approve, updated_at: Utc::now() };
    st.store.put(ADMIN_PK, "SETTINGS#AUTO", &s).await?;
    Ok(Json(s))
}

/// `POST /admin/coach-requests/{email}/invite`: approve, then the
/// invitation email goes out (or the response says why not, as elsewhere).
pub async fn invite_route(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let email = email.trim().to_lowercase();
    let found: Option<Request> = st.store.get(ADMIN_PK, &email_sk(&email)).await?;
    let Some(r) = found else { return Err(ApiError::NotFound("request")); };
    if r.status == Status::Invited {
        return Err(bad_request("an invitation was already emailed to this address; revoke it on the invitations list if it went unused"));
    }
    let (invited, out) = approve(&st, r).await?;
    Ok(Json(json!({ "request": invited, "invite": out })))
}

pub async fn deny(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let email = email.trim().to_lowercase();
    let found: Option<Request> = st.store.get(ADMIN_PK, &email_sk(&email)).await?;
    let Some(r) = found else { return Err(ApiError::NotFound("request")); };
    if r.status == Status::Invited {
        return Err(bad_request("that email already has an invitation — revoke the invitation instead, and they can ask again"));
    }
    let mut x = r;
    x.denied();
    x.updated_at = Utc::now();
    x.ttl = ttl();
    st.store.put(ADMIN_PK, &email_sk(&email), &x).await?;
    Ok(Json(json!({ "email": email, "status": x.status })))
}
