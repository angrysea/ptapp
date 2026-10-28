//! The coach's alert inbox (LG-7): pain reported on a workout, and runs of
//! missed workouts. Pain alerts are filed when the log is saved (and pushed
//! to the coach); missed-workout alerts are found when the coach's roster
//! is summarized (routes/clients.rs), which already reads each client's
//! recent calendar.

use axum::extract::{Path, State};
use axum::Json;
use chrono::{NaiveDate, Utc};
use ptcore::model::{Alert, AlertKind, RosterEntry, ScheduledWorkout, WorkoutLog};
use ptcore::stats::missed_streak;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::AppState;

/// Alert after this many missed workouts in a row, unless the coach says otherwise.
const DEFAULT_MISSED_IN_A_ROW: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlertSettings {
    pub missed_in_a_row: u32,
}

fn pk(coach: &str) -> String {
    format!("COACH#{coach}")
}

fn sk(id: &str) -> String {
    format!("ALERT#{id}")
}

pub async fn settings(st: &AppState, coach: &str) -> ApiResult<AlertSettings> {
    Ok(st.store.get(&pk(coach), "ALERTSETTINGS").await?.unwrap_or(AlertSettings { missed_in_a_row: DEFAULT_MISSED_IN_A_ROW }))
}

/// Files a pain alert for a saved log that reports pain, and tells the coach.
pub async fn on_log(st: &AppState, scope: &ClientScope, log: &WorkoutLog) -> ApiResult<()> {
    let Some(check) = log.check.as_ref().filter(|c| c.pain) else { return Ok(()) };
    let coach = &scope.client.coach_id;
    let where_ = check.pain_location.as_deref().map(str::trim).filter(|w| !w.is_empty());
    let alert = Alert {
        id: format!("pain.{}", log.id),
        kind: AlertKind::Pain,
        client_id: scope.client.user_id.clone(),
        client_name: scope.client.name.clone(),
        date: log.date,
        text: match where_ {
            Some(w) => format!("Reported pain ({w}) on {}", log.workout_name),
            None => format!("Reported pain on {}", log.workout_name),
        },
        log_id: Some(log.id.clone()),
        done: false,
        created_at: Utc::now(),
    };
    if st.store.put_new(&pk(coach), &sk(&alert.id), &alert).await? {
        let url = format!("#/c/{}/logs/{}", alert.client_id, log.id);
        super::notify::notify_user(st, coach, &alert.client_name, &alert.text, &url, &format!("alert-{}", alert.client_id)).await;
    }
    Ok(())
}

/// Files a missed-workouts alert for a client whose latest run of missed
/// workouts reaches the coach's threshold. One alert per run.
pub async fn check_missed(
    st: &AppState,
    coach: &str,
    threshold: u32,
    client: &RosterEntry,
    sched: &[ScheduledWorkout],
    logs: &[WorkoutLog],
    today: NaiveDate,
) -> ApiResult<()> {
    let Some((run, start)) = missed_streak(sched, logs, today) else { return Ok(()) };
    if run < threshold {
        return Ok(());
    }
    let latest = sched.iter().filter(|s| s.date < today).map(|s| s.date).max().unwrap_or(start);
    let alert = Alert {
        id: format!("missed.{}.{start}", client.client_id),
        kind: AlertKind::Missed,
        client_id: client.client_id.clone(),
        client_name: client.name.clone(),
        date: latest,
        text: format!("Missed {run} workouts in a row (since {})", start.format("%b %-d")),
        log_id: None,
        done: false,
        created_at: Utc::now(),
    };
    st.store.put_new(&pk(coach), &sk(&alert.id), &alert).await?;
    Ok(())
}

/// `GET /alerts`: open alerts, newest first, and the coach's settings.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let mut open: Vec<Alert> = st.store.query_prefix(&pk(coach), "ALERT#").await?;
    open.retain(|a| !a.done);
    open.sort_by_key(|a| std::cmp::Reverse(a.created_at));
    Ok(Json(json!({ "alerts": open, "settings": settings(&st, coach).await? })))
}

/// `POST /alerts/{id}/done`: the coach has dealt with it.
pub async fn done(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    let mut a: Alert = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("alert"))?;
    a.done = true;
    st.store.put(&pk(coach), &sk(&id), &a).await?;
    Ok(Json(json!({ "ok": true })))
}

/// `PUT /alerts/settings`.
pub async fn put_settings(State(st): State<AppState>, ctx: Ctx, Json(body): Json<AlertSettings>) -> ApiResult<Json<AlertSettings>> {
    let coach = ctx.require_coach()?;
    if !(1..=7).contains(&body.missed_in_a_row) {
        return Err(bad_request("alert after 1-7 missed workouts"));
    }
    st.store.put(&pk(coach), "ALERTSETTINGS", &body).await?;
    Ok(Json(body))
}
