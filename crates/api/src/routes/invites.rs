//! ON-1: coach invites a client by email or link; client accepts after
//! creating a Cognito account.

use aws_sdk_dynamodb::types::{Delete, Put, TransactWriteItem, Update};
use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::{Invite, Profile, Role, RosterEntry, UnitSystem};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{Caller, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::{key, keyed, s};
use crate::util::invite_code;
use crate::AppState;

const INVITE_DAYS: i64 = 14;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInvite {
    email: String,
    client_name: String,
}

pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(body): Json<CreateInvite>) -> ApiResult<Json<Invite>> {
    let coach_id = ctx.require_coach()?.to_string();
    let email = body.email.trim().to_lowercase();
    if !email.contains('@') {
        return Err(bad_request("a valid email is required"));
    }
    let now = Utc::now();
    let invite = Invite {
        code: invite_code(),
        coach_id: coach_id.clone(),
        coach_name: ctx.profile.business_name.clone().unwrap_or_else(|| ctx.profile.name.clone()),
        email,
        client_name: body.client_name.trim().to_string(),
        created_at: now,
        accepted_by: None,
        ttl: now.timestamp() + INVITE_DAYS * 24 * 3600,
    };
    let invite_pk = format!("INVITE#{}", invite.code);
    let coach_pk = format!("COACH#{coach_id}");
    st.store
        .batch_put(vec![
            keyed(&invite_pk, "INVITE", &invite)?,
            keyed(&coach_pk, &format!("INVITE#{}", invite.code), &invite)?,
        ])
        .await?;
    Ok(Json(invite))
}

pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Invite>>> {
    let coach_id = ctx.require_coach()?;
    let now = Utc::now().timestamp();
    let invites: Vec<Invite> = st.store.query_prefix(&format!("COACH#{coach_id}"), "INVITE#").await?;
    Ok(Json(invites.into_iter().filter(|i| i.accepted_by.is_none() && i.ttl > now).collect()))
}

pub async fn revoke(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let coach_id = ctx.require_coach()?;
    // Codes are stored uppercase; peek/accept already normalize, so revoke must too.
    let code = code.to_uppercase();
    let inv: Option<Invite> = st.store.get(&format!("INVITE#{code}"), "INVITE").await?;
    if inv.is_some_and(|i| i.coach_id == coach_id) {
        st.store.delete(&format!("INVITE#{code}"), "INVITE").await?;
        st.store.delete(&format!("COACH#{coach_id}"), &format!("INVITE#{code}")).await?;
    }
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

/// Lets the join screen show "Join <coach>'s team" before accepting.
pub async fn peek(State(st): State<AppState>, _caller: Caller, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let inv = open_invite(&st, &code).await?;
    Ok(Json(json!({ "coachName": inv.coach_name, "clientName": inv.client_name, "email": inv.email })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Accept {
    name: Option<String>,
}

pub async fn accept(
    State(st): State<AppState>,
    caller: Caller,
    Path(code): Path<String>,
    Json(body): Json<Accept>,
) -> ApiResult<Json<Profile>> {
    let inv = open_invite(&st, &code).await?;
    let now = Utc::now();
    let name = body
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| inv.client_name.clone())
        .trim()
        .to_string();
    let profile = Profile {
        user_id: caller.sub.clone(),
        role: Role::Client,
        coach_id: inv.coach_id.clone(),
        name: name.clone(),
        email: if caller.email.is_empty() { inv.email.clone() } else { caller.email.clone() },
        units: UnitSystem::Metric,
        sex: None,
        birth_date: None,
        business_name: None,
        created_at: now,
    };
    let roster = RosterEntry { client_id: caller.sub.clone(), name, email: profile.email.clone(), joined_at: now };
    let coach_pk = format!("COACH#{}", inv.coach_id);
    let t = &st.store.table;

    // All-or-nothing: the profile must not exist yet and the invite must still be open.
    let items = vec![
        TransactWriteItem::builder()
            .put(
                Put::builder()
                    .table_name(t)
                    .set_item(Some(keyed(&format!("USER#{}", caller.sub), "PROFILE", &profile)?))
                    .condition_expression("attribute_not_exists(pk)")
                    .build()
                    .map_err(|e| ApiError::Internal(e.to_string()))?,
            )
            .build(),
        TransactWriteItem::builder()
            .put(
                Put::builder()
                    .table_name(t)
                    .set_item(Some(keyed(&coach_pk, &format!("CLIENT#{}", caller.sub), &roster)?))
                    .build()
                    .map_err(|e| ApiError::Internal(e.to_string()))?,
            )
            .build(),
        TransactWriteItem::builder()
            .update(
                Update::builder()
                    .table_name(t)
                    .set_key(Some(key(&format!("INVITE#{}", inv.code), "INVITE")))
                    .update_expression("SET acceptedBy = :sub")
                    .condition_expression("attribute_exists(pk) AND (attribute_not_exists(acceptedBy) OR attribute_type(acceptedBy, :null))")
                    .expression_attribute_values(":sub", s(&caller.sub))
                    .expression_attribute_values(":null", s("NULL"))
                    .build()
                    .map_err(|e| ApiError::Internal(e.to_string()))?,
            )
            .build(),
        TransactWriteItem::builder()
            .delete(
                Delete::builder()
                    .table_name(t)
                    .set_key(Some(key(&coach_pk, &format!("INVITE#{}", inv.code))))
                    .build()
                    .map_err(|e| ApiError::Internal(e.to_string()))?,
            )
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
