//! Assigning programs to a client's calendar and logging workouts
//! (WB-4, LG-1, LG-3, LG-4, FB-2).

use aws_sdk_dynamodb::types::AttributeValue;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use ptcore::model::{Assignment, Program, ScheduledWorkout, WorkoutLog};
use ptcore::schedule::{end_date, materialize};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::keyed;
use crate::util::{new_id, parse_date, today};
use crate::AppState;

/// Highest sort-key suffix; ids and dates only use ASCII below '~'.
const HIGH: &str = "~";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssignBody {
    program_id: String,
    start_date: String,
}

pub async fn assign(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<AssignBody>,
) -> ApiResult<Json<Assignment>> {
    let coach = ctx.require_coach()?.to_string();
    let scope = client_scope(&st, &ctx, &cid).await?;
    let program: Program = st
        .store
        .get(&format!("COACH#{coach}"), &format!("PROGRAM#{}", body.program_id))
        .await?
        .ok_or(ApiError::NotFound("program"))?;
    let start = parse_date(&body.start_date)?;
    let assignment = Assignment {
        id: new_id(),
        program_id: program.id.clone(),
        program_name: program.name.clone(),
        start_date: start,
        end_date: end_date(start, program.weeks),
        created_at: Utc::now(),
    };
    let pk = scope.pk();
    let mut items = vec![keyed(&pk, &format!("ASSIGN#{}", assignment.id), &assignment)?];
    for (date, idx) in materialize(&program, start) {
        let pw = &program.workouts[idx];
        let sw = ScheduledWorkout {
            id: new_id(),
            date,
            assignment_id: assignment.id.clone(),
            program_name: program.name.clone(),
            week: pw.week,
            day: pw.day,
            workout: pw.workout.clone(),
        };
        items.push(keyed(&pk, &format!("SCHED#{date}#{}", sw.id), &sw)?);
    }
    st.store.batch_put(items).await?;
    Ok(Json(assignment))
}

pub async fn list_assignments(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Vec<Assignment>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mut a: Vec<Assignment> = st.store.query_prefix(&scope.pk(), "ASSIGN#").await?;
    a.sort_by_key(|x| std::cmp::Reverse(x.start_date));
    Ok(Json(a))
}

/// Removes the assignment and its not-yet-logged workouts from today on.
/// Past workouts stay so adherence history is preserved.
pub async fn unassign(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, aid)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    let from = format!("SCHED#{}", today());
    let future: Vec<ScheduledWorkout> = st.store.query_between(&pk, &from, &format!("SCHED#{HIGH}"), true).await?;
    let logged: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{}", today()), &format!("LOG#{HIGH}"), true).await?;
    let keys: Vec<(String, String)> = future
        .into_iter()
        .filter(|s| s.assignment_id == aid)
        .filter(|s| !logged.iter().any(|l| l.scheduled_id.as_deref() == Some(s.id.as_str())))
        .map(|s| (pk.clone(), format!("SCHED#{}#{}", s.date, s.id)))
        .collect();
    let removed = keys.len();
    st.store.batch_delete(keys).await?;
    st.store.delete(&pk, &format!("ASSIGN#{aid}")).await?;
    Ok(Json(json!({ "removed": removed })))
}

#[derive(Deserialize)]
pub struct Range {
    from: Option<String>,
    to: Option<String>,
}

impl Range {
    /// Defaults to one week back through four weeks ahead; capped at ~6 months.
    fn resolve(&self) -> ApiResult<(chrono::NaiveDate, chrono::NaiveDate)> {
        let from = match &self.from {
            Some(f) => parse_date(f)?,
            None => today() - Duration::days(7),
        };
        let to = match &self.to {
            Some(t) => parse_date(t)?,
            None => today() + Duration::days(28),
        };
        if to < from || (to - from).num_days() > 190 {
            return Err(bad_request("date range must be ascending and at most 190 days"));
        }
        Ok((from, to))
    }
}

pub async fn schedule(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<Json<Vec<ScheduledWorkout>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let (from, to) = r.resolve()?;
    let items = st
        .store
        .query_between(&scope.pk(), &format!("SCHED#{from}"), &format!("SCHED#{to}#{HIGH}"), true)
        .await?;
    Ok(Json(items))
}

pub async fn logs(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Query(r): Query<Range>,
) -> ApiResult<Json<Vec<WorkoutLog>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let (from, to) = r.resolve()?;
    let items = st
        .store
        .query_between(&scope.pk(), &format!("LOG#{from}"), &format!("LOG#{to}#{HIGH}"), true)
        .await?;
    Ok(Json(items))
}

/// Idempotent upsert keyed by a client-generated id. Offline devices replay
/// their queue here; the newest edit (by client timestamp) wins and an older
/// replay gets 409 with the server copy.
pub async fn put_log(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, lid)): Path<(String, String)>,
    Json(mut log): Json<WorkoutLog>,
) -> ApiResult<Json<WorkoutLog>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if log.id != lid {
        return Err(bad_request("log id in body does not match the URL"));
    }
    if lid.len() > 64 || !lid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(bad_request("log id must be 1-64 characters of [A-Za-z0-9-]"));
    }
    let now = Utc::now();
    if log.updated_at > now + Duration::minutes(5) {
        log.updated_at = now; // a phone clock far in the future must not win forever
    }
    let pk = scope.pk();
    let sk = format!("LOG#{}#{}", log.date, log.id);
    let mut item = keyed(&pk, &sk, &log)?;
    let ver = log.updated_at.timestamp_millis();
    item.insert("ver".into(), AttributeValue::N(ver.to_string()));

    let res = st
        .store
        .ddb
        .put_item()
        .table_name(&st.store.table)
        .set_item(Some(item))
        .condition_expression("attribute_not_exists(pk) OR ver < :v")
        .expression_attribute_values(":v", AttributeValue::N(ver.to_string()))
        .send()
        .await;
    match res {
        Ok(_) => Ok(Json(log)),
        Err(e) if e.as_service_error().is_some_and(|se| se.is_conditional_check_failed_exception()) => {
            let current: Option<WorkoutLog> = st.store.get(&pk, &sk).await?;
            Err(ApiError::Conflict(serde_json::to_value(current).unwrap_or(Value::Null)))
        }
        Err(e) => Err(e.into()),
    }
}
