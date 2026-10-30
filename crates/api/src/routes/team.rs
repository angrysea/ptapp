//! The head coach's team of assistant coaches (ptcore::team): inviting them
//! with a title and permissions, assigning them clients, changing or
//! removing their access. Only the head coach manages the team; an
//! assistant reads their own access from `/me/team`.

use axum::extract::{Path, State};
use axum::Json;
use ptcore::model::{Invite, InviteKind, Profile, RosterEntry};
use ptcore::team::{clean_perms, clean_title, Member, Perm, MAX_TEAM};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::routes::invites::{issue, new_invite, open_invites, revoke_where};
use crate::AppState;

fn member_sk(uid: &str) -> String {
    format!("ASSISTANT#{uid}")
}

async fn members(st: &AppState, coach: &str) -> ApiResult<Vec<Member>> {
    let mut ms: Vec<Member> = st.store.query_prefix(&format!("COACH#{coach}"), "ASSISTANT#").await?;
    ms.sort_by_key(|m| m.name.to_lowercase());
    Ok(ms)
}

/// `GET /team`: the assistants and the invitations not yet accepted.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let invites: Vec<Invite> = open_invites(&st, &format!("COACH#{coach}")).await?.into_iter().filter(|i| i.kind == InviteKind::Assistant).collect();
    Ok(Json(json!({ "members": members(&st, coach).await?, "invites": invites })))
}

#[derive(Deserialize)]
pub struct InviteIn {
    email: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    perms: Vec<Perm>,
}

/// `POST /team/invites`: invite an assistant by email, with a title (the
/// default "Assistant Coach") and permissions; emailed like client invites.
pub async fn invite(State(st): State<AppState>, ctx: Ctx, Json(body): Json<InviteIn>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let pending = open_invites(&st, &format!("COACH#{coach}")).await?.into_iter().filter(|i| i.kind == InviteKind::Assistant).count();
    if members(&st, coach).await?.len() + pending >= MAX_TEAM {
        return Err(bad_request(format!("a team has up to {MAX_TEAM} assistants")));
    }
    let mut inv = new_invite(InviteKind::Assistant, coach, &ctx.profile.name, &body.email, &body.name)?;
    inv.title = Some(clean_title(body.title.as_deref()).map_err(bad_request)?);
    inv.perms = clean_perms(body.perms);
    issue(&st, &inv).await?;
    Ok(Json(super::admin::with_email_status(&st, &inv).await?))
}

/// `DELETE /team/invites/{code}`.
pub async fn revoke_invite(State(st): State<AppState>, ctx: Ctx, Path(code): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?.to_string();
    revoke_where(&st, &code, |i| i.kind == InviteKind::Assistant && i.coach_id == coach).await?;
    Ok(Json(json!({ "revoked": code })))
}

#[derive(Deserialize)]
pub struct MemberIn {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    perms: Vec<Perm>,
    #[serde(default)]
    clients: Vec<String>,
}

/// `PUT /team/{uid}`: an assistant's title, permissions and assigned clients
/// (only the head coach's own clients can be assigned).
pub async fn update(State(st): State<AppState>, ctx: Ctx, Path(uid): Path<String>, Json(body): Json<MemberIn>) -> ApiResult<Json<Member>> {
    let coach = ctx.require_coach()?;
    let pk = format!("COACH#{coach}");
    let mut m: Member = st.store.get(&pk, &member_sk(&uid)).await?.ok_or(ApiError::NotFound("assistant"))?;
    let roster: Vec<RosterEntry> = st.store.query_prefix(&pk, "CLIENT#").await?;
    let mut clients = body.clients;
    clients.sort();
    clients.dedup();
    if let Some(stranger) = clients.iter().find(|c| !roster.iter().any(|r| &r.client_id == *c)) {
        return Err(bad_request(format!("{stranger} isn't one of your clients")));
    }
    m.title = clean_title(body.title.as_deref()).map_err(bad_request)?;
    m.perms = clean_perms(body.perms);
    m.clients = clients;
    st.store.put(&pk, &member_sk(&uid), &m).await?;
    Ok(Json(m))
}

/// `DELETE /team/{uid}`: the assistant loses all access at once (their
/// account stays, with nothing to see); what they made stays with the coach.
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(uid): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let pk = format!("COACH#{coach}");
    st.store.get::<Member>(&pk, &member_sk(&uid)).await?.ok_or(ApiError::NotFound("assistant"))?;
    st.store.delete(&pk, &member_sk(&uid)).await?;
    Ok(Json(json!({ "removed": uid })))
}

/// `GET /me/team`: an assistant's own title, permissions and clients, and
/// their head coach.
pub async fn mine(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let Some(m) = &ctx.member else { return Err(ApiError::Forbidden("assistants only")) };
    let head: Option<Profile> = st.store.get(&format!("USER#{}", ctx.profile.coach_id), "PROFILE").await?;
    Ok(Json(json!({ "member": m, "headCoach": head.map(|h| json!({ "userId": h.user_id, "name": h.name, "businessName": h.business_name })) })))
}
