//! A client's 1RMs (WB-5, ON-4 test lifts), which program loads written as
//! a percentage resolve against (ptcore::strength). Coach and client can
//! both record or change one, and a finished workout whose sets estimate
//! higher raises it (`raise_from_log`); any change re-weighs the client's
//! upcoming workouts.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use ptcore::model::{ScheduledWorkout, WorkoutLog};
use ptcore::stats::strength_trends;
use ptcore::strength::{raised_maxes, resolve_workout, uses_pct_of, Max, MaxSource};
use serde::Deserialize;
use serde_json::{json, Value};

use ptcore::team::Perm;
use crate::auth::{client_scope_as, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::keyed;
use crate::AppState;

const HIGH: &str = "~";

/// How far back logged sets are searched for e1RM estimates.
const ESTIMATE_DAYS: i64 = 182;

fn max_sk(exercise_id: &str) -> String {
    format!("MAX#{exercise_id}")
}

/// The client's maxes, in kg by exercise id.
pub async fn maxes_by_exercise(st: &AppState, client_pk: &str) -> ApiResult<HashMap<String, f64>> {
    let maxes: Vec<Max> = st.store.query_prefix(client_pk, "MAX#").await?;
    Ok(maxes.into_iter().map(|m| (m.exercise_id, m.kg)).collect())
}

/// Re-weighs %1RM loads of `exercise_id` in workouts from today on that
/// haven't been logged yet. Returns how many workouts changed.
async fn reweigh_upcoming(st: &AppState, scope: &ClientScope, exercise_id: &str) -> ApiResult<usize> {
    let pk = scope.pk();
    let today = scope.today();
    let upcoming: Vec<ScheduledWorkout> = st.store.query_between(&pk, &format!("SCHED#{today}"), &format!("SCHED#{HIGH}"), true).await?;
    let logged: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{today}"), &format!("LOG#{HIGH}"), true).await?;
    let maxes = maxes_by_exercise(st, &pk).await?;
    let mut items = Vec::new();
    for mut sw in upcoming {
        if !uses_pct_of(&sw.workout, exercise_id) || logged.iter().any(|l| l.scheduled_id.as_deref() == Some(sw.id.as_str())) {
            continue;
        }
        if resolve_workout(&mut sw.workout, &maxes, scope.client.units) {
            items.push(keyed(&pk, &format!("SCHED#{}#{}", sw.date, sw.id), &sw)?);
        }
    }
    let changed = items.len();
    st.store.batch_put(items).await?;
    Ok(changed)
}

/// After a workout is logged: every recorded max its working sets beat is
/// raised to the new estimate (source "estimated"), and upcoming workouts
/// re-weighed. Returns the exercises raised.
pub async fn raise_from_log(st: &AppState, scope: &ClientScope, log: &WorkoutLog) -> ApiResult<Vec<String>> {
    let pk = scope.pk();
    let maxes = maxes_by_exercise(st, &pk).await?;
    let mut raised = vec![];
    for (exercise_id, exercise_name, kg) in raised_maxes(log, &maxes) {
        let max = Max {
            exercise_id: exercise_id.clone(),
            exercise_name,
            kg: (kg * 10.0).round() / 10.0,
            source: MaxSource::Estimated,
            date: log.date,
            set_by: scope.actor_role(),
            updated_at: Utc::now(),
        };
        st.store.put(&pk, &max_sk(&exercise_id), &max).await?;
        reweigh_upcoming(st, scope, &exercise_id).await?;
        raised.push(exercise_id);
    }
    Ok(raised)
}

/// `GET /clients/{cid}/maxes`: recorded maxes, plus the best e1RM per
/// exercise from the last six months of logs, to offer as estimates.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Programming, Perm::Progress]).await?;
    let pk = scope.pk();
    let mut maxes: Vec<Max> = st.store.query_prefix(&pk, "MAX#").await?;
    maxes.sort_by_key(|m| m.exercise_name.to_lowercase());
    let from = scope.today() - Duration::days(ESTIMATE_DAYS);
    let logs: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{from}"), &format!("LOG#{HIGH}"), true).await?;
    let estimates: Vec<Value> = strength_trends(&logs)
        .into_iter()
        .filter_map(|t| {
            let best = t.points.iter().max_by(|a, b| a.e1rm_kg.total_cmp(&b.e1rm_kg))?;
            Some(json!({ "exerciseId": t.exercise_id, "exerciseName": t.exercise_name, "kg": best.e1rm_kg, "date": best.date }))
        })
        .collect();
    Ok(Json(json!({ "maxes": maxes, "estimates": estimates })))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MaxIn {
    exercise_name: String,
    kg: f64,
    source: MaxSource,
    date: Option<NaiveDate>,
}

/// `PUT /clients/{cid}/maxes/{exerciseId}`: records a max and re-weighs the
/// upcoming workouts that use it.
pub async fn put(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, exercise_id)): Path<(String, String)>,
    Json(body): Json<MaxIn>,
) -> ApiResult<Json<Value>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Programming]).await?;
    if !(body.kg > 0.0 && body.kg < 1000.0) {
        return Err(bad_request("a max must be between 0 and 1000 kg"));
    }
    let name = body.exercise_name.trim();
    if name.is_empty() || name.chars().count() > 100 {
        return Err(bad_request("exercise name must be 1-100 characters"));
    }
    let max = Max {
        exercise_id: exercise_id.clone(),
        exercise_name: name.to_string(),
        kg: (body.kg * 1000.0).round() / 1000.0,
        source: body.source,
        date: body.date.unwrap_or_else(|| scope.today()),
        set_by: scope.actor_role(),
        updated_at: Utc::now(),
    };
    st.store.put(&scope.pk(), &max_sk(&exercise_id), &max).await?;
    let updated = reweigh_upcoming(&st, &scope, &exercise_id).await?;
    Ok(Json(json!({ "max": max, "updatedWorkouts": updated })))
}

/// `DELETE /clients/{cid}/maxes/{exerciseId}`: upcoming %1RM loads of that
/// exercise go back to "pick a weight".
pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path((cid, exercise_id)): Path<(String, String)>) -> ApiResult<Json<Value>> {
    let scope = client_scope_as(&st, &ctx, &cid, &[Perm::Programming]).await?;
    if st.store.get::<Max>(&scope.pk(), &max_sk(&exercise_id)).await?.is_none() {
        return Err(ApiError::NotFound("max"));
    }
    st.store.delete(&scope.pk(), &max_sk(&exercise_id)).await?;
    let updated = reweigh_upcoming(&st, &scope, &exercise_id).await?;
    Ok(Json(json!({ "updatedWorkouts": updated })))
}
