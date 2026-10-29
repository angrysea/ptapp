//! Request queues: someone without an invitation asks for one, and signup
//! itself stays invitation-only. There are two queues, run the same way:
//!
//! * coach requests: the landing page's form lands in the admin's queue
//!   (`ADMIN` / `REQUEST#{email}`); approving creates the same coach
//!   invitation an admin makes by hand (routes/admin.rs);
//! * client requests: a coach's own sign-up link (`#/request/{coachId}`,
//!   which they can put on their website) lands in that coach's queue
//!   (`COACH#{coach}` / `REQUEST#{email}`); approving creates the same client
//!   invitation the coach sends from Clients (routes/invites.rs).
//!
//! The anonymous POSTs are the only doors in: an email and a name, one record
//! per email, expiring through DynamoDB `ttl` after 30 days. How they are
//! guarded is in ptcore::requests; API Gateway also throttles the routes
//! (template.yaml). Each queue owner can switch on auto-approve, but only
//! while the Turnstile human check is set up.

use std::time::Duration;

use aws_sdk_dynamodb::types::{AttributeValue, ReturnValue};
use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use ptcore::model::{Invite, InviteKind, Profile, Role, RosterEntry};
use ptcore::requests::{
    sender_key, valid_email, Request, Settings, Status, MAX_NAME, MAX_PENDING, MAX_PENDING_PER_COACH, MAX_PER_SENDER_HOUR, MIN_FILL_MS,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Ctx, SourceIp};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::admin::{APP_NAME, with_email_status};
use crate::routes::invites::{ADMIN_PK, issue, new_invite, open_invites};
use crate::store::key;
use crate::AppState;

const SITEVERIFY: &str = "https://challenges.cloudflare.com/turnstile/v0/siteverify";

/// A pending request sits in the queue this long if nothing happens.
const REQUEST_TTL_DAYS: i64 = 30;

/// Whose queue a request goes to, and what approving it sends.
struct Queue {
    /// The partition holding the queue, its settings and the invitations.
    pk: String,
    settings_sk: &'static str,
    kind: InviteKind,
    /// For client invitations: the coach, as the invitation names them.
    coach_id: String,
    coach_name: String,
    /// Each sender's hourly count is kept per kind of queue.
    rate: &'static str,
    max_pending: usize,
    /// Who hears about new requests (the coach); the admin reads the queue.
    notify: Option<String>,
}

impl Queue {
    fn admin() -> Queue {
        Queue {
            pk: ADMIN_PK.to_string(),
            settings_sk: "SETTINGS#AUTO",
            kind: InviteKind::Coach,
            coach_id: String::new(),
            coach_name: APP_NAME.to_string(),
            rate: "coach-request",
            max_pending: MAX_PENDING,
            notify: None,
        }
    }

    fn of_coach(coach: &Profile) -> Queue {
        Queue {
            pk: format!("COACH#{}", coach.user_id),
            settings_sk: "SETTINGS#REQUESTS",
            kind: InviteKind::Client,
            coach_id: coach.user_id.clone(),
            // As in an invitation the coach sends by hand (invites::create).
            coach_name: coach.business_name.clone().filter(|b| !b.trim().is_empty()).unwrap_or_else(|| coach.name.clone()),
            rate: "client-request",
            max_pending: MAX_PENDING_PER_COACH,
            notify: Some(coach.user_id.clone()),
        }
    }

    /// The coach a public link names, if they can take requests: an active
    /// (not suspended) coach account.
    async fn public_coach(st: &AppState, coach_id: &str) -> ApiResult<Queue> {
        let p: Option<Profile> = st.store.get(&format!("USER#{coach_id}"), "PROFILE").await?;
        match p {
            Some(p) if p.role == Role::Coach && p.suspended_at.is_none() => Ok(Queue::of_coach(&p)),
            _ => Err(ApiError::NotFound("coach")),
        }
    }
}

fn email_sk(email: &str) -> String {
    format!("REQUEST#{email}")
}

/// Fresh expiry on every transition, like an invite's ttl.
fn ttl() -> i64 {
    Utc::now().timestamp() + REQUEST_TTL_DAYS * 24 * 3600
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

/// Counts one request from `ip` (its `sender_key`: IPv6 by /64) in the
/// current hour, for this kind of queue; whether it is still within
/// `MAX_PER_SENDER_HOUR`. The counters expire after two hours.
async fn within_sender_limit(st: &AppState, rate: &str, ip: &str, now: DateTime<Utc>) -> ApiResult<bool> {
    let out = st
        .store
        .ddb
        .update_item()
        .table_name(&st.store.table)
        .set_key(Some(key(&format!("RATE#{rate}#{}", sender_key(ip)), &format!("HOUR#{}", now.format("%Y-%m-%dT%H")))))
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
/// entry. Used by the owner's approve route and by auto-approve.
async fn approve(st: &AppState, q: &Queue, req: Request) -> ApiResult<(Request, Value)> {
    // The name was typed by an anonymous visitor: it stays in the queue for
    // the owner but never goes into an email we send to someone else.
    let inv: Invite = new_invite(q.kind, &q.coach_id, &q.coach_name, &req.email, "")?;
    issue(st, &inv).await?;
    let out = with_email_status(st, &inv).await?;
    let mut r = req;
    r.invited(inv.code);
    r.updated_at = Utc::now();
    r.ttl = ttl();
    st.store.put(&q.pk, &email_sk(&r.email), &r).await?;
    Ok((r, out))
}

/// Whether `email` needs no request: already invited (an open invitation
/// from this queue's owner) or, in a coach's queue, already their client.
async fn already_in(st: &AppState, q: &Queue, email: &str) -> ApiResult<bool> {
    if open_invites(st, &q.pk).await?.iter().any(|i| i.email == email) {
        return Ok(true);
    }
    if q.kind == InviteKind::Client {
        let roster: Vec<RosterEntry> = st.store.query_prefix(&q.pk, "CLIENT#").await?;
        return Ok(roster.iter().any(|r| r.email.trim().eq_ignore_ascii_case(email)));
    }
    Ok(false)
}

/// Takes one anonymous request into `q`. Whatever happens to it, a
/// well-formed request gets the same answer, so the form can't be used to
/// find out who asked, was invited or is a client. Only a malformed email or
/// an over-long name is refused, which says nothing about anyone else.
async fn submit(st: &AppState, q: &Queue, ip: Option<String>, body: RequestIn) -> ApiResult<Json<Value>> {
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
        tracing::info!(queue = %q.pk, "request dropped: hidden field filled");
        return Ok(received);
    }
    // Too many from one sender this hour (checked before calling Cloudflare).
    if let Some(ip) = ip.as_deref()
        && !within_sender_limit(st, q.rate, ip, now).await?
    {
        tracing::info!(queue = %q.pk, "request dropped: sender over {MAX_PER_SENDER_HOUR} an hour");
        return Ok(received);
    }
    // A person: Turnstile when it's set up, else the page's own timing (too
    // fast, or no timing at all, is a script).
    let person = match st.turnstile_secret.as_deref() {
        Some(secret) => is_human(st, secret, &body.turnstile, ip.as_deref()).await?,
        None => body.elapsed_ms.is_some_and(|ms| ms >= MIN_FILL_MS),
    };
    if !person {
        tracing::info!(queue = %q.pk, "request dropped: failed the human check");
        return Ok(received);
    }
    let found: Option<Request> = st.store.get(&q.pk, &email_sk(&email)).await?;
    match &found {
        // Invited already, or already waiting: nothing changes.
        Some(r) if r.status != Status::Denied => return Ok(received),
        // Denied recently: the answer stands for now.
        Some(r) if !r.may_ask_again(now) => return Ok(received),
        _ => {}
    }
    if already_in(st, q, &email).await? {
        return Ok(received);
    }
    let all: Vec<Request> = st.store.query_prefix(&q.pk, "REQUEST#").await?;
    if all.iter().filter(|r| r.status == Status::Pending).count() >= q.max_pending {
        tracing::warn!(queue = %q.pk, "request dropped: {} already pending", q.max_pending);
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
    let settings: Option<Settings> = st.store.get(&q.pk, q.settings_sk).await?;
    // Auto-approve only behind Turnstile: without it the form would be open
    // signup, emailing whoever a script names.
    let auto = settings.is_some_and(|s| s.auto_approve) && st.turnstile_secret.is_some();
    if auto {
        // The invitation goes to the address by email; the visitor learns
        // nothing more than anyone else does.
        approve(st, q, r).await?;
    } else {
        st.store.put(&q.pk, &email_sk(&email), &r).await?;
    }
    if let Some(coach) = &q.notify {
        let text = if auto { format!("{email} asked to train with you and was sent an invitation") } else { format!("{email} asked to train with you") };
        super::notify::notify_user(st, coach, APP_NAME, &text, "#/", &format!("client-request-{email}")).await;
    }
    Ok(received)
}

/// The queue, newest first, with each request's status.
async fn list(st: &AppState, q: &Queue) -> ApiResult<Json<Vec<Request>>> {
    let mut out: Vec<Request> = st.store.query_prefix(&q.pk, "REQUEST#").await?;
    out.sort_by_key(|r| std::cmp::Reverse(r.requested_at));
    Ok(Json(out))
}

/// The queue settings, and whether the human check is set up (the owner is
/// offered auto-approve only then).
async fn settings_get(st: &AppState, q: &Queue) -> ApiResult<Json<Value>> {
    let s: Option<Settings> = st.store.get(&q.pk, q.settings_sk).await?;
    Ok(settings_out(st, q, s.unwrap_or_else(Settings::off)))
}

fn settings_out(st: &AppState, q: &Queue, s: Settings) -> Json<Value> {
    Json(json!({ "autoApprove": s.auto_approve, "updatedAt": s.updated_at, "humanCheck": st.turnstile_secret.is_some(), "maxPending": q.max_pending }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsIn {
    auto_approve: bool,
}

async fn settings_put(st: &AppState, q: &Queue, body: SettingsIn) -> ApiResult<Json<Value>> {
    if body.auto_approve && st.turnstile_secret.is_none() {
        return Err(bad_request("auto-approve needs the human check: set up Cloudflare Turnstile first (docs/DEPLOY.md)"));
    }
    let s = Settings { auto_approve: body.auto_approve, updated_at: Utc::now() };
    st.store.put(&q.pk, q.settings_sk, &s).await?;
    Ok(settings_out(st, q, s))
}

/// Approve one request by hand; the invitation email goes out (or the
/// response says why not, as elsewhere).
async fn invite_one(st: &AppState, q: &Queue, email: &str) -> ApiResult<Json<Value>> {
    let email = email.trim().to_lowercase();
    let found: Option<Request> = st.store.get(&q.pk, &email_sk(&email)).await?;
    let Some(r) = found else { return Err(ApiError::NotFound("request")); };
    if r.status == Status::Invited {
        return Err(bad_request("an invitation was already emailed to this address; revoke it on the invitations list if it went unused"));
    }
    let (invited, out) = approve(st, q, r).await?;
    Ok(Json(json!({ "request": invited, "invite": out })))
}

async fn deny_one(st: &AppState, q: &Queue, email: &str) -> ApiResult<Json<Value>> {
    let email = email.trim().to_lowercase();
    let found: Option<Request> = st.store.get(&q.pk, &email_sk(&email)).await?;
    let Some(r) = found else { return Err(ApiError::NotFound("request")); };
    if r.status == Status::Invited {
        return Err(bad_request("that email already has an invitation — revoke the invitation instead, and they can ask again"));
    }
    let mut x = r;
    x.denied();
    x.updated_at = Utc::now();
    x.ttl = ttl();
    st.store.put(&q.pk, &email_sk(&email), &x).await?;
    Ok(Json(json!({ "email": email, "status": x.status })))
}

// ---------------------------------------------------------------- coach requests (the admin's queue)

/// `POST /public/coach-requests` (no sign-in): the landing page's form.
pub async fn request(State(st): State<AppState>, SourceIp(ip): SourceIp, Json(body): Json<RequestIn>) -> ApiResult<Json<Value>> {
    submit(&st, &Queue::admin(), ip, body).await
}

/// `GET /admin/coach-requests`.
pub async fn queue(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Request>>> {
    ctx.require_admin()?;
    list(&st, &Queue::admin()).await
}

pub async fn get_settings(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    settings_get(&st, &Queue::admin()).await
}

pub async fn put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<SettingsIn>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    settings_put(&st, &Queue::admin(), body).await
}

/// `POST /admin/coach-requests/{email}/invite`.
pub async fn invite_route(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    invite_one(&st, &Queue::admin(), &email).await
}

/// `POST /admin/coach-requests/{email}/deny`.
pub async fn deny(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    deny_one(&st, &Queue::admin(), &email).await
}

// ---------------------------------------------------------------- client requests (each coach's queue)

/// `GET /public/join/{coachId}` (no sign-in): who the sign-up link is for.
pub async fn coach_page(State(st): State<AppState>, Path(coach): Path<String>) -> ApiResult<Json<Value>> {
    let q = Queue::public_coach(&st, &coach).await?;
    Ok(Json(json!({ "coachId": q.coach_id, "coachName": q.coach_name })))
}

/// `POST /public/join/{coachId}` (no sign-in): a would-be client's request.
pub async fn client_request(
    State(st): State<AppState>,
    SourceIp(ip): SourceIp,
    Path(coach): Path<String>,
    Json(body): Json<RequestIn>,
) -> ApiResult<Json<Value>> {
    let q = Queue::public_coach(&st, &coach).await?;
    submit(&st, &q, ip, body).await
}

/// `GET /requests`: the coach's own queue.
pub async fn coach_queue(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Request>>> {
    ctx.require_coach()?;
    list(&st, &Queue::of_coach(&ctx.profile)).await
}

pub async fn coach_get_settings(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    settings_get(&st, &Queue::of_coach(&ctx.profile)).await
}

pub async fn coach_put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<SettingsIn>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    settings_put(&st, &Queue::of_coach(&ctx.profile), body).await
}

/// `POST /requests/{email}/invite`.
pub async fn coach_invite(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    invite_one(&st, &Queue::of_coach(&ctx.profile), &email).await
}

/// `POST /requests/{email}/deny`.
pub async fn coach_deny(State(st): State<AppState>, ctx: Ctx, Path(email): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    deny_one(&st, &Queue::of_coach(&ctx.profile), &email).await
}
