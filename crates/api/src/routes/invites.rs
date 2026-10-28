//! ON-1: coach invites a client by email or link; client accepts after
//! creating a Cognito account on the app's own sign-up page, which fills in
//! the email from the invitation (`public`). Admins invite coaches the same
//! way, by email (routes/admin.rs); there is no open coach signup.

use aws_sdk_dynamodb::types::{Delete, Put, TransactWriteItem, Update};
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{CoachEntry, Invite, InviteKind, Profile, Role, RosterEntry, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::{key, keyed, s, Item};
use crate::util::invite_code;
use crate::AppState;

const INVITE_DAYS: i64 = 14;

/// Partition holding the admin's coach invites and coach list.
pub const ADMIN_PK: &str = "ADMIN";

/// Where an invite is listed for its sender: the coach's partition for
/// client invites, the admin partition for coach invites.
fn listing_pk(inv: &Invite) -> String {
    match inv.kind {
        InviteKind::Client => format!("COACH#{}", inv.coach_id),
        InviteKind::Coach => ADMIN_PK.to_string(),
    }
}

/// A new, open invite for `email` (validated and lowercased).
pub fn new_invite(kind: InviteKind, coach_id: &str, coach_name: &str, email: &str, name: &str) -> ApiResult<Invite> {
    let email = email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(bad_request("a valid email is required"));
    }
    let now = Utc::now();
    Ok(Invite {
        code: invite_code(),
        kind,
        coach_id: coach_id.to_string(),
        coach_name: coach_name.to_string(),
        email,
        client_name: name.trim().to_string(),
        created_at: now,
        accepted_by: None,
        ttl: now.timestamp() + INVITE_DAYS * 24 * 3600,
    })
}

/// Stores the invite under its code and in its sender's list.
pub async fn issue(st: &AppState, invite: &Invite) -> ApiResult<()> {
    st.store
        .batch_put(vec![
            keyed(&format!("INVITE#{}", invite.code), "INVITE", invite)?,
            keyed(&listing_pk(invite), &format!("INVITE#{}", invite.code), invite)?,
        ])
        .await
}

/// Open invites listed under `pk`.
pub async fn open_invites(st: &AppState, pk: &str) -> ApiResult<Vec<Invite>> {
    let now = Utc::now().timestamp();
    let invites: Vec<Invite> = st.store.query_prefix(pk, "INVITE#").await?;
    Ok(invites.into_iter().filter(|i| i.accepted_by.is_none() && i.ttl > now).collect())
}

/// Deletes an invite if `owns` accepts it; unknown codes are a no-op.
pub async fn revoke_where(st: &AppState, code: &str, owns: impl Fn(&Invite) -> bool) -> ApiResult<()> {
    // Codes are stored uppercase; peek/accept already normalize, so revoke must too.
    let code = code.to_uppercase();
    let inv: Option<Invite> = st.store.get(&format!("INVITE#{code}"), "INVITE").await?;
    if let Some(inv) = inv.filter(|i| owns(i)) {
        st.store.delete(&format!("INVITE#{code}"), "INVITE").await?;
        st.store.delete(&listing_pk(&inv), &format!("INVITE#{code}")).await?;
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    email: String,
    client_name: String,
}

/// A coach invites a client: the invitation is emailed (mail.rs), and the
/// response says whether it went, so the coach can share the link if not.
pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(body): Json<CreateInvite>) -> ApiResult<Json<Value>> {
    let coach_id = ctx.require_coach()?;
    let coach_name = ctx.profile.business_name.clone().unwrap_or_else(|| ctx.profile.name.clone());
    let invite = new_invite(InviteKind::Client, coach_id, &coach_name, &body.email, &body.client_name)?;
    issue(&st, &invite).await?;
    Ok(Json(super::admin::with_email_status(&st, &invite).await?))
}

/// Sends a pending client invitation's email again.
pub async fn resend(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let coach_id = ctx.require_coach()?;
    let code = code.to_uppercase();
    let invite = open_invites(&st, &format!("COACH#{coach_id}"))
        .await?
        .into_iter()
        .find(|i| i.code == code)
        .ok_or(ApiError::NotFound("invite"))?;
    Ok(Json(super::admin::with_email_status(&st, &invite).await?))
}

pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Invite>>> {
    let coach_id = ctx.require_coach()?;
    Ok(Json(open_invites(&st, &format!("COACH#{coach_id}")).await?))
}

pub async fn revoke(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let coach_id = ctx.require_coach()?;
    revoke_where(&st, &code, |i| i.kind == InviteKind::Client && i.coach_id == coach_id).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn open_invite(st: &AppState, code: &str) -> ApiResult<Invite> {
    let inv: Invite = st
        .store
        .get(&format!("INVITE#{}", code.to_uppercase()), "INVITE")
        .await?
        .ok_or(ApiError::NotFound("invite"))?;
    if inv.accepted_by.is_some() || inv.ttl < Utc::now().timestamp() {
        return Err(ApiError::NotFound("invite"));
    }
    Ok(inv)
}

/// The app's sign-up page (before any sign-in) reads the invitation here to
/// fill in, and lock, the email address. Open invitations only: once
/// accepted or expired, the link stops working. The code is the secret (10
/// random characters, single use, 14 days), and it only reveals the address
/// it was sent to.
pub async fn public(State(st): State<AppState>, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let inv = open_invite(&st, &code).await?;
    Ok(Json(json!({ "kind": inv.kind, "coachName": inv.coach_name, "clientName": inv.client_name, "email": inv.email })))
}

/// Lets the join screen show "Join <coach>'s team" (or the coach-invite
/// wording) before accepting.
pub async fn peek(State(st): State<AppState>, _caller: Caller, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let inv = open_invite(&st, &code).await?;
    Ok(Json(json!({ "kind": inv.kind, "coachName": inv.coach_name, "clientName": inv.client_name, "email": inv.email })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accept {
    name: Option<String>,
    /// The person ticked "I agree" to how their data is used (privacy NFR).
    #[serde(default)]
    consent: bool,
}

fn build<T, E: std::fmt::Display>(r: Result<T, E>) -> ApiResult<T> {
    r.map_err(|e| ApiError::Internal(e.to_string()))
}

pub async fn accept(
    State(st): State<AppState>,
    caller: Caller,
    Path(code): Path<String>,
    Json(body): Json<Accept>,
) -> ApiResult<Json<Profile>> {
    if !body.consent {
        return Err(bad_request("agree to how your data is used to create your account"));
    }
    let inv = open_invite(&st, &code).await?;
    // The code alone is not enough: a forwarded or leaked link must not let
    // someone else join as this client or coach.
    if !caller.email_verified || !caller.email.trim().eq_ignore_ascii_case(&inv.email) {
        return Err(ApiError::Forbidden("this invite is for a different email address; sign in with the one you were invited with"));
    }
    let now = Utc::now();
    let name = body
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| inv.client_name.clone())
        .trim()
        .to_string();
    if name.is_empty() {
        return Err(bad_request("name is required"));
    }
    let (role, coach_id) = match inv.kind {
        InviteKind::Client => (Role::Client, inv.coach_id.clone()),
        // Each coach is a tenant: coach_id is their own id.
        InviteKind::Coach => (Role::Coach, caller.sub.clone()),
    };
    let profile = Profile {
        user_id: caller.sub.clone(),
        role,
        coach_id,
        name: name.clone(),
        email: inv.email.clone(),
        units: UnitSystem::default(),
        sex: None,
        birth_date: None,
        business_name: None,
        time_zone: None,
        consented_at: Some(now),
        suspended_at: None,
        created_at: now,
    };
    // The new member's entry in the sender's list: the coach's roster or the
    // admin's coach list.
    let member: Item = match inv.kind {
        InviteKind::Client => {
            let roster = RosterEntry { client_id: caller.sub.clone(), name, email: profile.email.clone(), joined_at: now };
            keyed(&listing_pk(&inv), &format!("CLIENT#{}", caller.sub), &roster)?
        }
        InviteKind::Coach => {
            let entry = CoachEntry { coach_id: caller.sub.clone(), name, email: profile.email.clone(), joined_at: now, suspended_at: None };
            keyed(&listing_pk(&inv), &format!("COACH#{}", caller.sub), &entry)?
        }
    };
    let t = &st.store.table;

    // All-or-nothing: the profile must not exist yet and the invite must still be open.
    let items = vec![
        TransactWriteItem::builder()
            .put(build(
                Put::builder()
                    .table_name(t)
                    .set_item(Some(keyed(&format!("USER#{}", caller.sub), "PROFILE", &profile)?))
                    .condition_expression("attribute_not_exists(pk)")
                    .build(),
            )?)
            .build(),
        TransactWriteItem::builder()
            .put(build(Put::builder().table_name(t).set_item(Some(member)).build())?)
            .build(),
        TransactWriteItem::builder()
            .update(build(
                Update::builder()
                    .table_name(t)
                    .set_key(Some(key(&format!("INVITE#{}", inv.code), "INVITE")))
                    .update_expression("SET acceptedBy = :sub")
                    .condition_expression("attribute_exists(pk) AND (attribute_not_exists(acceptedBy) OR attribute_type(acceptedBy, :null))")
                    .expression_attribute_values(":sub", s(&caller.sub))
                    .expression_attribute_values(":null", s("NULL"))
                    .build(),
            )?)
            .build(),
        TransactWriteItem::builder()
            .delete(build(
                Delete::builder()
                    .table_name(t)
                    .set_key(Some(key(&listing_pk(&inv), &format!("INVITE#{}", inv.code))))
                    .build(),
            )?)
            .build(),
    ];
    match st.store.ddb.transact_write_items().set_transact_items(Some(items)).send().await {
        Ok(_) => Ok(Json(profile)),
        Err(e) if e.as_service_error().is_some_and(|se| se.is_transaction_canceled_exception()) => {
            Err(ApiError::Conflict(json!({ "message": "invite already used, or this account is already set up" })))
        }
        Err(e) => Err(e.into()),
    }
}
