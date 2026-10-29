//! Daily habits (BZ-3; ptcore::habits). The coach or the client adds
//! habits; the client logs each day's value (through the offline outbox, so
//! a write is a plain idempotent PUT of the day's value).

use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use ptcore::habits::{editable, streak, Habit, HabitKind, HabitLog, MAX_HABITS};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::{new_id, parse_date};
use crate::AppState;

const HIGH: &str = "~";

fn sk(id: &str) -> String {
    format!("HABIT#{id}")
}

#[derive(Deserialize)]
pub struct DaysQuery {
    days: Option<i64>,
}

/// `GET /clients/{cid}/habits?days=14`: habits (archived too), each active
/// one's streak, and the logs of the last `days` days.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Query(q): Query<DaysQuery>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    let today = scope.today();
    let days = q.days.unwrap_or(14).clamp(1, 90);
    let mut habits: Vec<Habit> = st.store.query_prefix(&pk, "HABIT#").await?;
    habits.sort_by_key(|h| h.created_at);
    // Streaks need logs back to where they break; a year covers any real run.
    let since = today - Duration::days(366);
    let logs: Vec<HabitLog> = st.store.query_between(&pk, &format!("HABITLOG#{since}"), &format!("HABITLOG#{HIGH}"), true).await?;
    let streaks: serde_json::Map<String, Value> = habits.iter().filter(|h| h.active).map(|h| (h.id.clone(), json!(streak(h, &logs, today)))).collect();
    let from = today - Duration::days(days - 1);
    let recent: Vec<&HabitLog> = logs.iter().filter(|l| l.date >= from).collect();
    Ok(Json(json!({ "today": today, "habits": habits, "streaks": streaks, "logs": recent })))
}

#[derive(Deserialize)]
pub struct HabitIn {
    name: String,
    kind: HabitKind,
    #[serde(default)]
    target: f64,
    #[serde(default)]
    unit: Option<String>,
    #[serde(default)]
    active: Option<bool>,
}

/// `POST /clients/{cid}/habits` (coach or client).
pub async fn create(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>, Json(body): Json<HabitIn>) -> ApiResult<Json<Habit>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let existing: Vec<Habit> = st.store.query_prefix(&scope.pk(), "HABIT#").await?;
    if existing.iter().filter(|h| h.active).count() >= MAX_HABITS {
        return Err(bad_request(format!("up to {MAX_HABITS} habits at a time; archive one first")));
    }
    let mut h = Habit { id: new_id()[..12].to_string(), name: body.name, kind: body.kind, target: body.target, unit: body.unit, active: true, created_at: Utc::now() };
    h.validate().map_err(bad_request)?;
    st.store.put(&scope.pk(), &sk(&h.id), &h).await?;
    Ok(Json(h))
}

/// `PUT /clients/{cid}/habits/{hid}`: rename, retarget, archive or restore.
pub async fn update(State(st): State<AppState>, ctx: Ctx, Path((cid, hid)): Path<(String, String)>, Json(body): Json<HabitIn>) -> ApiResult<Json<Habit>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mut h: Habit = st.store.get(&scope.pk(), &sk(&hid)).await?.ok_or(ApiError::NotFound("habit"))?;
    (h.name, h.kind, h.target, h.unit) = (body.name, body.kind, body.target, body.unit);
    if let Some(active) = body.active {
        h.active = active;
    }
    h.validate().map_err(bad_request)?;
    st.store.put(&scope.pk(), &sk(&hid), &h).await?;
    Ok(Json(h))
}

#[derive(Deserialize)]
pub struct DayValue {
    value: f64,
}

/// `PUT /clients/{cid}/habits/{hid}/days/{date}`: the client's value for a
/// day (0 clears it). Today or the week before.
pub async fn log_day(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, hid, date)): Path<(String, String, String)>,
    Json(body): Json<DayValue>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client logs their habits"));
    }
    let date = parse_date(&date)?;
    if !editable(date, scope.today()) {
        return Err(bad_request("habits can be logged for today and the past week"));
    }
    if !(body.value >= 0.0 && body.value <= 1_000_000.0) {
        return Err(bad_request("value must be 0 or more"));
    }
    let h: Habit = st.store.get(&scope.pk(), &sk(&hid)).await?.ok_or(ApiError::NotFound("habit"))?;
    let value = if h.kind == HabitKind::Check { body.value.min(1.0).round() } else { body.value };
    let log_sk = format!("HABITLOG#{date}#{hid}");
    if value == 0.0 {
        st.store.delete(&scope.pk(), &log_sk).await?;
    } else {
        st.store.put(&scope.pk(), &log_sk, &HabitLog { habit_id: hid, date, value, updated_at: Utc::now() }).await?;
    }
    Ok(Json(json!({ "date": date, "value": value })))
}
