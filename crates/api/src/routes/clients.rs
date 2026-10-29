//! Coach roster, client detail, intake (ON-2), waiver (ON-3) and the
//! progress dashboard (LG-6).

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use futures::stream::{self, StreamExt};
use ptcore::model::{Assignment, Intake, ParqAnswer, Profile, RosterEntry, ScheduledWorkout, Waiver, WorkoutLog};
use ptcore::stats::{adherence, strength_trends, weekly_volume};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, save_profile, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{audit, new_id};
use crate::AppState;

/// Scheduled workouts and logs from `days` before `today` through `today`.
pub async fn recent(st: &AppState, pk: &str, today: NaiveDate, days: i64) -> ApiResult<(Vec<ScheduledWorkout>, Vec<WorkoutLog>)> {
    let from = today - Duration::days(days);
    let to = today;
    let (s_from, s_to) = (format!("SCHED#{from}"), format!("SCHED#{to}#~"));
    let (l_from, l_to) = (format!("LOG#{from}"), format!("LOG#{to}#~"));
    let sched = st.store.query_between(pk, &s_from, &s_to, true);
    let logs = st.store.query_between(pk, &l_from, &l_to, true);
    let (s, l) = tokio::try_join!(sched, logs)?;
    Ok((s, l))
}

async fn latest_waiver(st: &AppState, pk: &str) -> ApiResult<Option<Waiver>> {
    let mut w: Vec<Waiver> = st.store.query_prefix(pk, "WAIVER#").await?;
    Ok(w.pop())
}

const ROSTER_CONCURRENCY: usize = 8;

/// Roster with a 28-day adherence summary and flags the coach should act on.
pub async fn roster(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Value>>> {
    let coach = ctx.require_coach()?;
    let entries: Vec<RosterEntry> = st.store.query_prefix(&format!("COACH#{coach}"), "CLIENT#").await?;
    let threshold = super::alerts::settings(&st, coach).await?.missed_in_a_row;
    // Clients are summarized a few at a time (each takes five reads) so a big
    // roster doesn't fire hundreds of DynamoDB requests at once.
    let rows: Vec<ApiResult<Value>> = stream::iter(entries)
        .map(|e| {
            let st = st.clone();
            async move {
                let pk = format!("CLIENT#{}", e.client_id);
                // The client's own profile holds their time zone, so "missed" is
                // judged by their calendar, not UTC's.
                let profile: Option<Profile> = st.store.get(&format!("USER#{}", e.client_id), "PROFILE").await?;
                let today = profile.map_or_else(|| Utc::now().date_naive(), |p| p.today(Utc::now()));
                // `recent` stops at today, so look at assignments for anything ahead.
                let ((sched, logs), waiver, assignments) = tokio::try_join!(
                    recent(&st, &pk, today, 28),
                    latest_waiver(&st, &pk),
                    st.store.query_prefix::<Assignment>(&pk, "ASSIGN#"),
                )?;
                let a = adherence(&sched, &logs, today);
                // LG-7: a run of missed workouts becomes an alert (once per run).
                if let Err(err) = super::alerts::check_missed(&st, coach, threshold, &e, &sched, &logs, today).await {
                    tracing::warn!(client = %e.client_id, "missed-workout alert failed: {err:?}");
                }
                let last_workout = logs.iter().filter(|l| l.status.counts_as_done()).map(|l| l.date).max();
                let pain_since = today - Duration::days(14);
                let pain = logs
                    .iter()
                    .any(|l| l.date >= pain_since && l.check.as_ref().is_some_and(|c| c.pain));
                let upcoming = sched.iter().any(|s| s.date >= today) || assignments.iter().any(|a| a.end_date >= today);
                Ok(json!({
                    "client": e,
                    "adherence": a,
                    "lastWorkout": last_workout,
                    "painFlag": pain,
                    "waiverSigned": waiver.is_some(),
                    "parqFlagged": waiver.is_some_and(|w| w.parq_flagged),
                    "hasUpcoming": upcoming,
                }))
            }
        })
        .buffered(ROSTER_CONCURRENCY)
        .collect()
        .await;
    rows.into_iter().collect::<ApiResult<Vec<_>>>().map(Json)
}

pub async fn detail(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    let intake: Option<Intake> = st.store.get(&pk, "INTAKE").await?;
    let waiver = latest_waiver(&st, &pk).await?;
    audit(&st, &ctx, &scope, "view-client").await;
    Ok(Json(json!({ "profile": scope.client, "intake": intake, "waiver": waiver })))
}

pub async fn get_intake(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Option<Intake>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-intake").await;
    Ok(Json(st.store.get(&scope.pk(), "INTAKE").await?))
}

pub async fn put_intake(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(mut intake): Json<Intake>,
) -> ApiResult<Json<Intake>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("the client fills in their own intake"));
    }
    intake.updated_at = Some(Utc::now());
    st.store.put(&scope.pk(), "INTAKE", &intake).await?;
    // Sex and birth date drive the skinfold equations, so mirror them onto the profile.
    let mut p: Profile = scope.client;
    if intake.sex.is_some() {
        p.sex = intake.sex;
    }
    if intake.birth_date.is_some() {
        p.birth_date = intake.birth_date;
    }
    save_profile(&st, &p).await?;
    Ok(Json(intake))
}

pub async fn get_waiver(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Option<Waiver>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-waiver").await;
    Ok(Json(latest_waiver(&st, &scope.pk()).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SignWaiver {
    waiver_version: String,
    parq: Vec<ParqAnswer>,
    signed_name: String,
    agreed: bool,
}

/// Stores an immutable, timestamped signature record. Re-signing (e.g. a new
/// waiver version) adds another record; the latest one is current.
pub async fn sign_waiver(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<SignWaiver>,
) -> ApiResult<Json<Waiver>> {
    let scope: ClientScope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client can sign their waiver"));
    }
    if !body.agreed || body.signed_name.trim().len() < 2 {
        return Err(bad_request("type your full name and tick the box to sign"));
    }
    if body.parq.is_empty() {
        return Err(bad_request("answer the health screening questions"));
    }
    let now = Utc::now();
    let w = Waiver {
        id: new_id(),
        waiver_version: body.waiver_version,
        parq_flagged: body.parq.iter().any(|a| a.answer),
        parq: body.parq,
        signed_name: body.signed_name.trim().to_string(),
        signed_at: now,
        source_ip: ctx.caller.source_ip.clone(),
        user_agent: ctx.caller.user_agent.clone(),
    };
    let sk = format!("WAIVER#{}", now.to_rfc3339());
    if !st.store.put_new(&scope.pk(), &sk, &w).await? {
        return Err(ApiError::Conflict(Value::Null));
    }
    Ok(Json(w))
}

#[derive(Deserialize)]
pub struct DashQuery {
    weeks: Option<i64>,
}

pub async fn dashboard(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Query(q): Query<DashQuery>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    audit(&st, &ctx, &scope, "view-dashboard").await;
    let weeks = q.weeks.unwrap_or(8).clamp(1, 26);
    // Fetch at least 28 days so the "last28" adherence is always a 28-day
    // window; each reported window then filters explicitly.
    let today = scope.today();
    let (sched, logs) = recent(&st, &scope.pk(), today, (weeks * 7).max(28)).await?;
    let win_start = today - Duration::days(weeks * 7);
    let l28_start = today - Duration::days(28);
    let sched_w = sched.iter().filter(|s| s.date >= win_start).cloned().collect::<Vec<_>>();
    let logs_w = logs.iter().filter(|l| l.date >= win_start).cloned().collect::<Vec<_>>();
    let checks: Vec<Value> = logs_w
        .iter()
        .rev()
        .filter_map(|l| l.check.as_ref().map(|c| json!({ "date": l.date, "workoutName": l.workout_name, "check": c })))
        .take(20)
        .collect();
    Ok(Json(json!({
        "weeks": weeks,
        "adherence": adherence(&sched_w, &logs_w, today),
        "last28": adherence(
            &sched.iter().filter(|s| s.date >= l28_start).cloned().collect::<Vec<_>>(),
            &logs.iter().filter(|l| l.date >= l28_start).cloned().collect::<Vec<_>>(),
            today,
        ),
        "weeklyVolume": weekly_volume(&logs_w),
        "strength": strength_trends(&logs_w),
        "checks": checks,
    })))
}
