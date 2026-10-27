//! Admins onboard coaches; there is no open coach signup. A coach account
//! comes from either an admin's invite (accepted like a client invite, see
//! routes/invites.rs) or an approved request: a signed-in user asks to
//! coach, and approval turns their account into a coach account.
//!
//! The first admin bootstraps through ADMIN_EMAILS: a verified email on
//! that list may set up an admin account.

use aws_sdk_dynamodb::types::{Put, TransactWriteItem, Update};
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{CoachEntry, CoachRequest, Invite, InviteKind, Profile, RequestStatus, Role, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::invites::{issue, new_invite, open_invites, revoke_where, ADMIN_PK};
use crate::store::{key, keyed, s};
use crate::AppState;

/// Shown on the join screen of coach invites.
const APP_NAME: &str = "PT Coach";

fn request_sk(user_id: &str) -> String {
    format!("REQUEST#{user_id}")
}

fn build<T, E: std::fmt::Display>(r: Result<T, E>) -> ApiResult<T> {
    r.map_err(|e| ApiError::Internal(e.to_string()))
}

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
        units: UnitSystem::Metric,
        sex: None,
        birth_date: None,
        business_name: None,
        time_zone: None,
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

pub async fn create_invite(State(st): State<AppState>, ctx: Ctx, Json(body): Json<CoachInvite>) -> ApiResult<Json<Invite>> {
    ctx.require_admin()?;
    let invite = new_invite(InviteKind::Coach, "", APP_NAME, &body.email, &clean_name(&body.name)?)?;
    issue(&st, &invite).await?;
    Ok(Json(invite))
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

// ---------------------------------------------------------------- requests

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestIn {
    name: String,
    business_name: Option<String>,
    #[serde(default)]
    message: String,
}

/// A signed-in user without an account asks to coach. Submitting again
/// (e.g. after a decline) replaces their request and puts it back in review.
pub async fn submit_request(State(st): State<AppState>, caller: Caller, Json(body): Json<RequestIn>) -> ApiResult<Json<CoachRequest>> {
    if !caller.email_verified {
        return Err(ApiError::Forbidden("verify your email address first"));
    }
    if st.store.get::<Profile>(&format!("USER#{}", caller.sub), "PROFILE").await?.is_some() {
        return Err(ApiError::Conflict(json!({ "message": "account already set up" })));
    }
    let message = body.message.trim().to_string();
    if message.chars().count() > 2000 {
        return Err(bad_request("message must be at most 2000 characters"));
    }
    let request = CoachRequest {
        user_id: caller.sub.clone(),
        email: caller.email.trim().to_lowercase(),
        name: clean_name(&body.name)?,
        business_name: body.business_name.map(|b| b.trim().to_string()).filter(|b| !b.is_empty()),
        message,
        status: RequestStatus::Pending,
        created_at: Utc::now(),
        decided_at: None,
    };
    st.store.put(ADMIN_PK, &request_sk(&caller.sub), &request).await?;
    Ok(Json(request))
}

/// Pending requests first (oldest first, so none waits forever), then the
/// decided ones, newest first.
pub async fn requests(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<CoachRequest>>> {
    ctx.require_admin()?;
    let mut list: Vec<CoachRequest> = st.store.query_prefix(ADMIN_PK, "REQUEST#").await?;
    list.sort_by(|a, b| match (a.status == RequestStatus::Pending, b.status == RequestStatus::Pending) {
        (true, true) => a.created_at.cmp(&b.created_at),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => b.decided_at.cmp(&a.decided_at),
    });
    Ok(Json(list))
}

async fn pending_request(st: &AppState, uid: &str) -> ApiResult<CoachRequest> {
    let r: CoachRequest = st.store.get(ADMIN_PK, &request_sk(uid)).await?.ok_or(ApiError::NotFound("request"))?;
    if r.status != RequestStatus::Pending {
        return Err(ApiError::Conflict(json!({ "message": "this request was already decided" })));
    }
    Ok(r)
}

fn status_str(status: RequestStatus) -> String {
    serde_json::to_value(status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Marks a pending request decided (as a transaction item). The
/// condition makes a second decision, e.g. two admins at once, fail instead
/// of overwriting the first.
fn decide(st: &AppState, uid: &str, status: RequestStatus) -> ApiResult<Update> {
    build(
        Update::builder()
            .table_name(&st.store.table)
            .set_key(Some(key(ADMIN_PK, &request_sk(uid))))
            .update_expression("SET #s = :s, decidedAt = :at")
            .condition_expression("#s = :pending")
            .expression_attribute_names("#s", "status")
            .expression_attribute_values(":s", s(status_str(status)))
            .expression_attribute_values(":at", s(Utc::now().to_rfc3339()))
            .expression_attribute_values(":pending", s(status_str(RequestStatus::Pending)))
            .build(),
    )
}

/// Approval creates the coach account for the requester's own sign-in, adds
/// them to the coach list and closes the request, all or nothing.
pub async fn approve(State(st): State<AppState>, ctx: Ctx, Path(uid): Path<String>) -> ApiResult<Json<Profile>> {
    ctx.require_admin()?;
    let r = pending_request(&st, &uid).await?;
    let now = Utc::now();
    let profile = Profile {
        user_id: r.user_id.clone(),
        role: Role::Coach,
        coach_id: r.user_id.clone(),
        name: r.name.clone(),
        email: r.email.clone(),
        units: UnitSystem::Metric,
        sex: None,
        birth_date: None,
        business_name: r.business_name.clone(),
        time_zone: None,
        created_at: now,
    };
    let entry = CoachEntry { coach_id: r.user_id.clone(), name: r.name.clone(), email: r.email.clone(), joined_at: now };
    let t = &st.store.table;
    let items = vec![
        TransactWriteItem::builder()
            .put(build(
                Put::builder()
                    .table_name(t)
                    .set_item(Some(keyed(&format!("USER#{uid}"), "PROFILE", &profile)?))
                    .condition_expression("attribute_not_exists(pk)")
                    .build(),
            )?)
            .build(),
        TransactWriteItem::builder()
            .put(build(Put::builder().table_name(t).set_item(Some(keyed(ADMIN_PK, &format!("COACH#{uid}"), &entry)?)).build())?)
            .build(),
        TransactWriteItem::builder().update(decide(&st, &uid, RequestStatus::Approved)?).build(),
    ];
    match st.store.ddb.transact_write_items().set_transact_items(Some(items)).send().await {
        Ok(_) => Ok(Json(profile)),
        Err(e) if e.as_service_error().is_some_and(|se| se.is_transaction_canceled_exception()) => Err(ApiError::Conflict(
            json!({ "message": "this request was already decided, or the person already has an account" }),
        )),
        Err(e) => Err(e.into()),
    }
}

pub async fn decline(State(st): State<AppState>, ctx: Ctx, Path(uid): Path<String>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    pending_request(&st, &uid).await?;
    let item = TransactWriteItem::builder().update(decide(&st, &uid, RequestStatus::Declined)?).build();
    match st.store.ddb.transact_write_items().transact_items(item).send().await {
        Ok(_) => Ok(Json(json!({ "ok": true }))),
        Err(e) if e.as_service_error().is_some_and(|se| se.is_transaction_canceled_exception()) => {
            Err(ApiError::Conflict(json!({ "message": "this request was already decided" })))
        }
        Err(e) => Err(e.into()),
    }
}
