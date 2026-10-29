//! Coach request queue: the landing page lets a coach ask to join without
//! a link (docs/ARCHITECTURE.md); signup itself stays invitation-only —
//! approving a request creates and emails exactly the same single-use
//! invitation an admin makes by hand (routes/admin.rs, routes/invites.rs).
//! The anonymous POST is the only door in: it carries an email and a name,
//! holds one record per email (the sk is the email), and its records expire
//! through DynamoDB `ttl` after 30 days. How it is guarded is in
//! ptcore::requests; API Gateway also throttles the route (template.yaml).

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{Invite, InviteKind};
use ptcore::requests::{valid_email, Request, Settings, Status, MAX_NAME, MAX_PENDING, MIN_FILL_MS};
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
    /// The hidden field: people never see it, form-filling scripts fill it.
    #[serde(default)]
    website: String,
    /// Milliseconds from the page showing the form to sending it.
    #[serde(default)]
    elapsed_ms: Option<u64>,
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
/// Whatever happens to the request, a well-formed one gets the same answer,
/// so the form can't be used to find out who asked or was invited. Only a
/// malformed email or an over-long name is refused, which says nothing about
/// anyone else.
pub async fn request(State(st): State<AppState>, Json(body): Json<RequestIn>) -> ApiResult<Json<Value>> {
    let email = body.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(bad_request("a valid email is required (up to 80 characters)"));
    }
    let name = body.name.trim().to_string();
    if name.chars().count() > MAX_NAME {
        return Err(bad_request("a name is at most 100 characters"));
    }
    let received = Json(json!({ "status": "received" }));
    // Scripts: the hidden field is filled, or the form came back too fast (or
    // without the page's timing at all). Answer as usual and keep nothing.
    if !body.website.is_empty() || body.elapsed_ms.is_none_or(|ms| ms < MIN_FILL_MS) {
        tracing::info!("coach request dropped: looks automated");
        return Ok(received);
    }
    let now = Utc::now();
    let found: Option<Request> = st.store.get(ADMIN_PK, &email_sk(&email)).await?;
    match &found {
        // Invited already, or already waiting: nothing changes.
        Some(r) if r.status != Status::Denied => return Ok(received),
        // Denied recently: the answer stands for now.
        Some(r) if !r.may_ask_again(now) => return Ok(received),
        _ => {}
    }
    let all: Vec<Request> = st.store.query_prefix(ADMIN_PK, "REQUEST#").await?;
    if all.iter().filter(|r| r.status == Status::Pending).count() >= MAX_PENDING {
        tracing::warn!("coach request dropped: {MAX_PENDING} already pending");
        return Ok(received);
    }
    let mut r = found.unwrap_or(Request {
        email: email.clone(),
        name: name.clone(),
        status: Status::Pending,
        requested_at: now,
        updated_at: now,
        invite_code: None,
        ttl: ttl(),
    });
    // A denial that has run its course goes back on the queue with any new name.
    r.re_request();
    r.name = name;
    r.updated_at = now;
    r.ttl = ttl();
    let settings: Option<Settings> = st.store.get(ADMIN_PK, "SETTINGS#AUTO").await?;
    if settings.is_some_and(|s| s.auto_approve) {
        // The invitation goes to the address by email; the visitor learns
        // nothing more than anyone else does.
        approve(&st, r).await?;
    } else {
        st.store.put(ADMIN_PK, &email_sk(&email), &r).await?;
    }
    Ok(received)
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
