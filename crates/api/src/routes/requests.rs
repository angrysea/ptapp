//! Coach request queue: the landing page lets a coach ask to join without
//! a link (docs/ARCHITECTURE.md); signup itself stays invitation-only —
//! approving a request creates and emails exactly the same single-use
//! invitation an admin makes by hand (routes/admin.rs, routes/invites.rs).
//! The anonymous POST is the only door in: it carries an email and a name,
//! holds one record per email (the sk is the email), and its records expire
//! through DynamoDB `ttl` after 30 days. How it is guarded is in
//! ptcore::requests; API Gateway also throttles the route (template.yaml).

use std::time::Duration;

use aws_sdk_dynamodb::types::{AttributeValue, ReturnValue};
use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use ptcore::model::{Invite, InviteKind};
use ptcore::requests::{valid_email, Request, Settings, Status, MAX_NAME, MAX_PENDING, MAX_PER_SENDER_HOUR, MIN_FILL_MS};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Ctx, SourceIp};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::admin::{APP_NAME, with_email_status};
use crate::routes::invites::{ADMIN_PK, issue, new_invite};
use crate::store::key;
use crate::AppState;

const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// The Turnstile secret: TURNSTILE_SECRET as given (tests use Cloudflare's
/// published test secrets), else the SSM SecureString named by
/// TURNSTILE_PARAM. None when neither is set or the parameter can't be read.
pub async fn turnstile_secret(config: &aws_config::SdkConfig) -> Option<String> {
    if let Some(s) = std::env::var("TURNSTILE_SECRET").ok().filter(|s| !s.trim().is_empty()) {
        return Some(s);
    }
    let name = std::env::var("TURNSTILE_PARAM").ok().filter(|n| !n.is_empty())?;
    match aws_sdk_ssm::Client::new(config).get_parameter().name(&name).with_decryption(true).send().await {
        Ok(out) => out.parameter.and_then(|p| p.value).filter(|v| !v.trim().is_empty()),
        Err(e) => {
            tracing::warn!(param = %name, error = %aws_sdk_ssm::error::DisplayErrorContext(e), "Turnstile off: can't read the secret");
            None
        }
    }
}

/// Whether Cloudflare says the widget's `token` came from a person. An error
/// (Cloudflare unreachable) is a 500, so the visitor is told to try again
/// rather than being silently dropped.
async fn is_human(st: &AppState, secret: &str, token: &str, ip: Option<&str>) -> ApiResult<bool> {
    if token.is_empty() {
        return Ok(false);
    }
    let mut body = json!({ "secret": secret, "response": token });
    if let Some(ip) = ip {
        body["remoteip"] = json!(ip);
    }
    let res = st.http.post(SITEVERIFY).json(&body).timeout(Duration::from_secs(5)).send().await
        .map_err(|e| ApiError::Internal(format!("turnstile unreachable: {e}")))?;
    let out: Value = res.json().await.map_err(|e| ApiError::Internal(format!("turnstile answer: {e}")))?;
    if out["success"] != json!(true) {
        tracing::info!(codes = %out["error-codes"], "turnstile said no");
    }
    Ok(out["success"] == json!(true))
}

/// Counts one request from `ip` in the current hour; whether it is still
/// within `MAX_PER_SENDER_HOUR`. The counters expire after two hours.
async fn within_sender_limit(st: &AppState, ip: &str, now: DateTime<Utc>) -> ApiResult<bool> {
    let out = st
        .store
        .ddb
        .update_item()
        .table_name(&st.store.table)
        .set_key(Some(key(&format!("RATE#coach-request#{ip}"), &format!("HOUR#{}", now.format("%Y-%m-%dT%H")))))
        .update_expression("ADD n :one SET #ttl = :ttl")
        .expression_attribute_names("#ttl", "ttl")
        .expression_attribute_values(":one", AttributeValue::N("1".into()))
        .expression_attribute_values(":ttl", AttributeValue::N((now.timestamp() + 2 * 3600).to_string()))
        .return_values(ReturnValue::UpdatedNew)
        .send()
        .await?;
    let n = out.attributes().and_then(|a| a.get("n")).and_then(|v| v.as_n().ok()).and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
    Ok(n <= MAX_PER_SENDER_HOUR)
}

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
    /// The Turnstile widget's token, when the page shows the widget.
    #[serde(default)]
    turnstile: String,
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
pub async fn request(State(st): State<AppState>, SourceIp(ip): SourceIp, Json(body): Json<RequestIn>) -> ApiResult<Json<Value>> {
    let email = body.email.trim().to_lowercase();
    if !valid_email(&email) {
        return Err(bad_request("a valid email is required (up to 80 characters)"));
    }
    let name = body.name.trim().to_string();
    if name.chars().count() > MAX_NAME {
        return Err(bad_request("a name is at most 100 characters"));
    }
    let received = Json(json!({ "status": "received" }));
    let now = Utc::now();
    // Whatever trips a check below gets the usual answer and leaves nothing.
    // The hidden field is filled: a script.
    if !body.website.is_empty() {
        tracing::info!("coach request dropped: hidden field filled");
        return Ok(received);
    }
    // Too many from one sender this hour (checked before calling Cloudflare).
    if let Some(ip) = ip.as_deref()
        && !within_sender_limit(&st, ip, now).await?
    {
        tracing::info!("coach request dropped: sender over {MAX_PER_SENDER_HOUR} an hour");
        return Ok(received);
    }
    // A person: Turnstile when it's set up, else the page's own timing (too
    // fast, or no timing at all, is a script).
    let person = match st.turnstile_secret.as_deref() {
        Some(secret) => is_human(&st, secret, &body.turnstile, ip.as_deref()).await?,
        None => body.elapsed_ms.is_some_and(|ms| ms >= MIN_FILL_MS),
    };
    if !person {
        tracing::info!("coach request dropped: failed the human check");
        return Ok(received);
    }
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
    // Auto-approve only behind Turnstile: without it the form would be open
    // signup, emailing whoever a script names.
    if settings.is_some_and(|s| s.auto_approve) && st.turnstile_secret.is_some() {
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

/// The queue settings, and whether the human check is set up (the admin
/// page offers auto-approve only then).
fn settings_out(st: &AppState, s: Settings) -> Json<Value> {
    Json(json!({ "autoApprove": s.auto_approve, "updatedAt": s.updated_at, "humanCheck": st.turnstile_secret.is_some(), "maxPending": MAX_PENDING }))
}

pub async fn get_settings(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let s: Option<Settings> = st.store.get(ADMIN_PK, "SETTINGS#AUTO").await?;
    Ok(settings_out(&st, s.unwrap_or_else(Settings::off)))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIn {
    auto_approve: bool,
}

pub async fn put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<SettingsIn>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    if body.auto_approve && st.turnstile_secret.is_none() {
        return Err(bad_request("auto-approve needs the human check: set up Cloudflare Turnstile first (docs/DEPLOY.md)"));
    }
    let s = Settings { auto_approve: body.auto_approve, updated_at: Utc::now() };
    st.store.put(ADMIN_PK, "SETTINGS#AUTO", &s).await?;
    Ok(settings_out(&st, s))
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
