//! Service credits (ptcore::credits): a coach's plans, the grants a client
//! holds, and the uses that count them down. Other routes call `require`
//! before a client starts something that needs a credit, `spend` when the
//! service is delivered, and `refund` when it's undone (a cancelled session).

use aws_sdk_dynamodb::types::AttributeValue;
use axum::extract::{Path, State};
use axum::Json;
use chrono::{NaiveDate, Utc};
use ptcore::credits::{balances, grants_for, pick, Grant, LedgerEntry, Plan, Service, Settings};
use ptcore::model::Video;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::new_id;
use crate::AppState;

const SETTINGS_SK: &str = "CREDIT_SETTINGS";

fn plan_sk(id: &str) -> String {
    format!("PLAN#{id}")
}

fn grant_sk(id: &str) -> String {
    format!("CREDIT#{id}")
}

/// A coach's credit settings (enforcement off until they turn it on).
pub async fn settings_for(st: &AppState, coach_id: &str) -> ApiResult<Settings> {
    Ok(st.store.get(&format!("COACH#{coach_id}"), SETTINGS_SK).await?.unwrap_or_default())
}

async fn grants(st: &AppState, scope: &ClientScope) -> ApiResult<Vec<Grant>> {
    st.store.query_prefix(&scope.pk(), "CREDIT#").await
}

/// Credits held by requests not yet delivered: set videos awaiting review.
async fn held(st: &AppState, scope: &ClientScope) -> ApiResult<Vec<(Service, u32)>> {
    let videos: Vec<Video> = st.store.query_prefix(&scope.pk(), "VIDEO#").await?;
    let waiting = videos.iter().filter(|v| v.reviewed_at.is_none()).count() as u32;
    Ok(vec![(Service::VideoReview, waiting)])
}

async fn ledger(st: &AppState, scope: &ClientScope, service: Service, change: i64, unlimited: bool, reason: &str, grant_id: Option<String>) -> ApiResult<()> {
    let e = LedgerEntry { id: new_id()[..12].to_string(), service, change, unlimited, reason: reason.into(), grant_id, at: Utc::now() };
    st.store.put(&scope.pk(), &format!("LEDGER#{}#{}", e.at.to_rfc3339(), e.id), &e).await
}

fn out_of(service: Service, by_coach: bool) -> ApiError {
    ApiError::PaymentRequired(if by_coach {
        format!("this client has no {} left; grant them a plan or more credits first", service.label())
    } else {
        format!("you have no {} left; ask your coach about your plan", service.label())
    })
}

/// Refuses when the coach enforces credits and the client has none of
/// `service` free to use (counting ones held by requests in progress).
pub async fn require(st: &AppState, scope: &ClientScope, service: Service) -> ApiResult<()> {
    if !service.enforced() || !settings_for(st, &scope.client.coach_id).await?.enforce {
        return Ok(());
    }
    let today = scope.today();
    let b = balances(&grants(st, scope).await?, today, &held(st, scope).await?);
    match b.iter().find(|x| x.service == service) {
        Some(x) if x.available => Ok(()),
        _ => Err(out_of(service, scope.by_coach)),
    }
}

/// After a request that holds a credit has been recorded (a set video):
/// refuses if the holds now outnumber the credits. Two uploads started at
/// once can both pass `require`; this check, made after both are written,
/// sees both holds, so at most the credits there are get through.
pub async fn confirm_hold(st: &AppState, scope: &ClientScope, service: Service) -> ApiResult<()> {
    if !service.enforced() || !settings_for(st, &scope.client.coach_id).await?.enforce {
        return Ok(());
    }
    let b = balances(&grants(st, scope).await?, scope.today(), &held(st, scope).await?);
    match b.iter().find(|x| x.service == service) {
        Some(x) if x.remaining.is_none_or(|r| r >= x.held) => Ok(()),
        _ => Err(out_of(service, scope.by_coach)),
    }
}

/// Takes one credit of `service` for something delivered. With no credit
/// left: refused if `block` and the coach enforces credits, otherwise the
/// use goes unrecorded against a grant (and is noted in the ledger).
/// Returns the grant it came out of.
pub async fn spend(st: &AppState, scope: &ClientScope, service: Service, reason: &str, block: bool) -> ApiResult<Option<String>> {
    let today = scope.today();
    // Two uses at once can't both take the last credit: the write only
    // succeeds if `used` is still what was read.
    for _ in 0..5 {
        let all = grants(st, scope).await?;
        let Some(g) = pick(&all, service, today) else {
            if block && service.enforced() && settings_for(st, &scope.client.coach_id).await?.enforce {
                return Err(out_of(service, scope.by_coach));
            }
            if all.iter().any(|g| g.service == service) {
                ledger(st, scope, service, 0, false, &format!("{reason} (no credit left)"), None).await?;
            }
            return Ok(None);
        };
        let mut next = g.clone();
        next.used += 1;
        let ok = st.store.put_if_with(&scope.pk(), &grant_sk(&g.id), &next, "used = :u", &[(":u", AttributeValue::N(g.used.to_string()))]).await?;
        if ok {
            ledger(st, scope, service, -1, false, reason, Some(g.id.clone())).await?;
            return Ok(Some(g.id.clone()));
        }
    }
    Err(ApiError::Internal("credit use kept clashing".into()))
}

/// Gives back a credit taken from `grant_id` (a cancelled session).
pub async fn refund(st: &AppState, scope: &ClientScope, grant_id: &str, service: Service, reason: &str) -> ApiResult<()> {
    for _ in 0..5 {
        let Some(g) = st.store.get::<Grant>(&scope.pk(), &grant_sk(grant_id)).await? else { return Ok(()) };
        if g.used == 0 {
            return Ok(());
        }
        let next = Grant { used: g.used - 1, ..g.clone() };
        if st.store.put_if_with(&scope.pk(), &grant_sk(grant_id), &next, "used = :u", &[(":u", AttributeValue::N(g.used.to_string()))]).await? {
            return ledger(st, scope, service, 1, false, reason, Some(grant_id.into())).await;
        }
    }
    Err(ApiError::Internal("credit refund kept clashing".into()))
}

// ---------------------------------------------------------------- coach settings and plans

/// `GET /credits/settings`.
pub async fn get_settings(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Settings>> {
    let coach = ctx.require_coach()?;
    Ok(Json(settings_for(&st, coach).await?))
}

/// `PUT /credits/settings`: turn enforcement on or off for this coach's clients.
pub async fn put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<Settings>) -> ApiResult<Json<Settings>> {
    let coach = ctx.require_coach()?;
    st.store.put(&format!("COACH#{coach}"), SETTINGS_SK, &body).await?;
    Ok(Json(body))
}

/// `GET /plans`: the coach's plans, by name.
pub async fn list_plans(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Plan>>> {
    let coach = ctx.require_coach()?;
    let mut plans: Vec<Plan> = st.store.query_prefix(&format!("COACH#{coach}"), "PLAN#").await?;
    plans.sort_by_key(|p| p.name.to_lowercase());
    Ok(Json(plans))
}

/// `POST /plans`.
pub async fn create_plan(State(st): State<AppState>, ctx: Ctx, Json(mut p): Json<Plan>) -> ApiResult<Json<Plan>> {
    let coach = ctx.require_coach()?;
    let existing: Vec<Plan> = st.store.query_prefix(&format!("COACH#{coach}"), "PLAN#").await?;
    if existing.len() >= 50 {
        return Err(bad_request("up to 50 plans; delete one you no longer offer"));
    }
    p.id = new_id()[..12].to_string();
    p.validate().map_err(bad_request)?;
    p.updated_at = Some(Utc::now());
    st.store.put(&format!("COACH#{coach}"), &plan_sk(&p.id), &p).await?;
    Ok(Json(p))
}

/// `PUT /plans/{id}`: changes apply to future grants; credits already granted stay as they are.
pub async fn update_plan(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(mut p): Json<Plan>) -> ApiResult<Json<Plan>> {
    let coach = ctx.require_coach()?;
    let pk = format!("COACH#{coach}");
    st.store.get::<Plan>(&pk, &plan_sk(&id)).await?.ok_or(ApiError::NotFound("plan"))?;
    p.id = id;
    p.validate().map_err(bad_request)?;
    p.updated_at = Some(Utc::now());
    st.store.put(&pk, &plan_sk(&p.id), &p).await?;
    Ok(Json(p))
}

/// `DELETE /plans/{id}`: clients keep what they were granted.
pub async fn delete_plan(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    st.store.delete(&format!("COACH#{coach}"), &plan_sk(&id)).await?;
    Ok(Json(json!({ "deleted": id })))
}

// ---------------------------------------------------------------- a client's credits

/// `GET /clients/{cid}/credits`: balances per service, the grants, and the
/// latest uses. The client and their coach can both see it.
pub async fn get_client(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let today = scope.today();
    let mut all = grants(&st, &scope).await?;
    let b = balances(&all, today, &held(&st, &scope).await?);
    all.sort_by_key(|g| std::cmp::Reverse(g.granted_at));
    let history: Vec<LedgerEntry> = st.store.latest(&scope.pk(), "LEDGER#", 50).await?;
    let enforce = settings_for(&st, &scope.client.coach_id).await?.enforce;
    Ok(Json(json!({ "today": today, "enforced": enforce, "balances": b, "grants": all, "history": history })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantPlan {
    plan_id: String,
    #[serde(default)]
    expires_on: Option<NaiveDate>,
}

/// `POST /clients/{cid}/credits/plan`: the coach grants one of their plans.
/// `expiresOn` overrides the plan's length.
pub async fn grant_plan(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<GrantPlan>) -> ApiResult<Json<Vec<Grant>>> {
    let coach = ctx.require_coach()?.to_string();
    let scope = client_scope(&st, &ctx, &cid).await?;
    let plan: Plan = st.store.get(&format!("COACH#{coach}"), &plan_sk(&body.plan_id)).await?.ok_or(ApiError::NotFound("plan"))?;
    let today = scope.today();
    if body.expires_on.is_some_and(|d| d < today) {
        return Err(bad_request("the expiry date has passed"));
    }
    let gs = grants_for(&plan, today, Utc::now(), body.expires_on, || new_id()[..12].to_string());
    for g in &gs {
        st.store.put(&scope.pk(), &grant_sk(&g.id), g).await?;
        ledger(&st, &scope, g.service, g.total.unwrap_or(0) as i64, g.total.is_none(), &format!("{} plan", plan.name), Some(g.id.clone())).await?;
    }
    Ok(Json(gs))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantCredits {
    service: Service,
    /// `None`: unlimited.
    #[serde(default)]
    quantity: Option<u32>,
    #[serde(default)]
    expires_on: Option<NaiveDate>,
    #[serde(default)]
    note: Option<String>,
}

/// `POST /clients/{cid}/credits`: the coach adds credits of one service by hand.
pub async fn grant_credits(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<GrantCredits>) -> ApiResult<Json<Grant>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    if body.quantity.is_some_and(|q| q == 0 || q > ptcore::credits::MAX_QUANTITY) {
        return Err(bad_request("1-10000 credits, or unlimited"));
    }
    if body.expires_on.is_some_and(|d| d < scope.today()) {
        return Err(bad_request("the expiry date has passed"));
    }
    let note = body.note.map(|n| n.trim().chars().take(200).collect::<String>()).filter(|n| !n.is_empty());
    let g = Grant {
        id: new_id()[..12].to_string(),
        service: body.service,
        total: body.quantity,
        used: 0,
        expires_on: body.expires_on,
        plan_id: None,
        plan_name: None,
        note: note.clone(),
        granted_at: Utc::now(),
    };
    st.store.put(&scope.pk(), &grant_sk(&g.id), &g).await?;
    ledger(&st, &scope, g.service, g.total.unwrap_or(0) as i64, g.total.is_none(), note.as_deref().unwrap_or("Added by your coach"), Some(g.id.clone())).await?;
    Ok(Json(g))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditGrant {
    #[serde(default)]
    expires_on: Option<NaiveDate>,
}

/// `PUT /clients/{cid}/credits/{gid}`: the coach changes when a grant expires
/// (`null`: never).
pub async fn edit_grant(State(st): State<AppState>, ctx: Ctx, Path((cid, gid)): Path<(String, String)>, Json(body): Json<EditGrant>) -> ApiResult<Json<Grant>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mut g: Grant = st.store.get(&scope.pk(), &grant_sk(&gid)).await?.ok_or(ApiError::NotFound("credits"))?;
    g.expires_on = body.expires_on;
    st.store.put(&scope.pk(), &grant_sk(&gid), &g).await?;
    Ok(Json(g))
}

/// `DELETE /clients/{cid}/credits/{gid}`: the coach takes back what's left of a grant.
pub async fn revoke(State(st): State<AppState>, ctx: Ctx, Path((cid, gid)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    let g: Grant = st.store.get(&scope.pk(), &grant_sk(&gid)).await?.ok_or(ApiError::NotFound("credits"))?;
    st.store.delete(&scope.pk(), &grant_sk(&gid)).await?;
    let left = g.remaining().map(|r| -(r as i64)).unwrap_or(0);
    ledger(&st, &scope, g.service, left, g.total.is_none(), "Removed by your coach", Some(gid.clone())).await?;
    Ok(Json(json!({ "deleted": gid })))
}
