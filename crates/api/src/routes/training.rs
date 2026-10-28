//! Assigning programs to a client's calendar and logging workouts
//! (WB-4, LG-1, LG-3, LG-4, FB-2).

use std::collections::HashSet;

use aws_sdk_dynamodb::types::{AttributeValue, Delete, Put, TransactWriteItem};
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use ptcore::model::{Assignment, Program, ScheduledWorkout, WorkoutLog};
use ptcore::schedule::{end_date, materialize};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::{key, keyed, s};
use crate::util::{audit, new_id, parse_date};
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
    let end = end_date(start, program.weeks);
    // Two overlapping assignments would schedule the same dates twice and
    // inflate adherence denominators, so refuse before materializing.
    let existing: Vec<Assignment> = st.store.query_prefix(&scope.pk(), "ASSIGN#").await?;
    if let Some(clash) = existing.iter().find(|a| a.start_date <= end && start <= a.end_date) {
        return Err(bad_request(format!(
            "overlaps the {name} assignment ({from} to {to}); pick another start date or remove it first",
            name = clash.program_name,
            from = clash.start_date,
            to = clash.end_date,
        )));
    }
    // Workouts can outlive their assignment (unassign keeps today's and past
    // ones), so also refuse to put a second workout on a day that has one.
    let dates = materialize(&program, start);
    let new_days: HashSet<NaiveDate> = dates.iter().map(|(d, _)| *d).collect();
    let taken: Vec<ScheduledWorkout> = st
        .store
        .query_between(&scope.pk(), &format!("SCHED#{start}"), &format!("SCHED#{end}#{HIGH}"), true)
        .await?;
    if let Some(clash) = taken.iter().find(|s| new_days.contains(&s.date)) {
        return Err(bad_request(format!(
            "{date} already has {name} scheduled; pick a start date that avoids it",
            date = clash.date,
            name = clash.workout.name,
        )));
    }
    let assignment = Assignment {
        id: new_id(),
        program_id: program.id.clone(),
        program_name: program.name.clone(),
        start_date: start,
        end_date: end,
        created_at: Utc::now(),
    };
    let pk = scope.pk();
    let mut items = vec![keyed(&pk, &format!("ASSIGN#{}", assignment.id), &assignment)?];
    for (date, idx) in dates {
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

/// Removes the assignment and its not-yet-logged workouts from tomorrow on.
/// Today's workout stays so a client mid-workout keeps it; past workouts
/// stay too, preserving adherence history.
pub async fn unassign(
    State(st): State<AppState>,
    ctx: Ctx,
    Path((cid, aid)): Path<(String, String)>,
) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    let scope = client_scope(&st, &ctx, &cid).await?;
    let pk = scope.pk();
    let today = scope.today();
    let from = format!("SCHED#{}", today + Duration::days(1));
    let future: Vec<ScheduledWorkout> = st.store.query_between(&pk, &from, &format!("SCHED#{HIGH}"), true).await?;
    let logged: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{today}"), &format!("LOG#{HIGH}"), true).await?;
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
    /// Defaults to one week back through four weeks ahead of the client's
    /// `today`; capped at ~6 months.
    fn resolve(&self, today: NaiveDate) -> ApiResult<(NaiveDate, NaiveDate)> {
        let from = match &self.from {
            Some(f) => parse_date(f)?,
            None => today - Duration::days(7),
        };
        let to = match &self.to {
            Some(t) => parse_date(t)?,
            None => today + Duration::days(28),
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
    audit(&st, &ctx, &scope, "view-schedule").await;
    let (from, to) = r.resolve(scope.today())?;
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
    audit(&st, &ctx, &scope, "view-logs").await;
    let (from, to) = r.resolve(scope.today())?;
    let items = st
        .store
        .query_between(&scope.pk(), &format!("LOG#{from}"), &format!("LOG#{to}#{HIGH}"), true)
        .await?;
    Ok(Json(items))
}

/// Where a log's live copy is filed. The log's sort key includes its date,
/// so this pointer (`LOGREF#{id}`) is how a write finds a copy filed under an
/// older date without scanning the client's whole log history.
#[derive(Serialize, Deserialize)]
struct LogRef {
    date: NaiveDate,
    ver: i64,
}

fn log_sk(date: NaiveDate, id: &str) -> String {
    format!("LOG#{date}#{id}")
}

/// The stored copy of a log, wherever it is filed, for a 409 body.
async fn current_log(st: &AppState, pk: &str, log: &WorkoutLog) -> ApiResult<Option<WorkoutLog>> {
    let date = match st.store.get::<LogRef>(pk, &format!("LOGREF#{}", log.id)).await? {
        Some(r) => r.date,
        None => log.date,
    };
    st.store.get(pk, &log_sk(date, &log.id)).await
}

fn build<T, E: std::fmt::Display>(r: Result<T, E>) -> ApiResult<T> {
    r.map_err(|e| ApiError::Internal(e.to_string()))
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
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client logs their workouts"));
    }
    if log.id != lid {
        return Err(bad_request("log id in body does not match the URL"));
    }
    if lid.len() > 64 || !lid.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return Err(bad_request("log id must be 1-64 characters of [A-Za-z0-9-]"));
    }
    log.clean_feedback().map_err(bad_request)?;
    let now = Utc::now();
    if log.updated_at > now + Duration::minutes(5) {
        log.updated_at = now; // a phone clock far in the future must not win forever
    }
    let pk = scope.pk();
    let sk = log_sk(log.date, &log.id);
    let ref_sk = format!("LOGREF#{}", log.id);
    let ver = log.updated_at.timestamp_millis();
    let t = &st.store.table;

    // One log id must have exactly one live item: if an offline device edited
    // the log's date, the old-dated copy is replaced in the same transaction.
    // The pointer is the version gate. Its condition matches exactly what was
    // read, so a write that lands in between cancels this one, and the loop
    // re-reads and tries again (or reports 409 if that write was newer).
    for _ in 0..3 {
        let prior: Option<LogRef> = st.store.get(&pk, &ref_sk).await?;
        if prior.as_ref().is_some_and(|p| p.ver > ver) {
            let current = current_log(&st, &pk, &log).await?;
            return Err(ApiError::Conflict(serde_json::to_value(current).unwrap_or(Value::Null)));
        }
        let mut item = keyed(&pk, &sk, &log)?;
        item.insert("ver".into(), AttributeValue::N(ver.to_string()));
        // Logs written before pointers existed have none; their own `ver` still guards them.
        let put_log = Put::builder()
            .table_name(t)
            .set_item(Some(item))
            .condition_expression("attribute_not_exists(pk) OR ver <= :v")
            .expression_attribute_values(":v", AttributeValue::N(ver.to_string()));
        let mut put_ref = Put::builder().table_name(t).set_item(Some(keyed(&pk, &ref_sk, &LogRef { date: log.date, ver })?));
        put_ref = match &prior {
            None => put_ref.condition_expression("attribute_not_exists(pk)"),
            Some(p) => put_ref
                .condition_expression("ver = :pv AND #d = :pd")
                .expression_attribute_names("#d", "date")
                .expression_attribute_values(":pv", AttributeValue::N(p.ver.to_string()))
                .expression_attribute_values(":pd", s(p.date.to_string())),
        };
        let mut items = vec![
            TransactWriteItem::builder().put(build(put_log.build())?).build(),
            TransactWriteItem::builder().put(build(put_ref.build())?).build(),
        ];
        if let Some(p) = prior.as_ref().filter(|p| p.date != log.date) {
            let stale = Delete::builder().table_name(t).set_key(Some(key(&pk, &log_sk(p.date, &log.id))));
            items.push(TransactWriteItem::builder().delete(build(stale.build())?).build());
        }
        match st.store.ddb.transact_write_items().set_transact_items(Some(items)).send().await {
            Ok(_) => return Ok(Json(log)),
            Err(e) if e.as_service_error().is_some_and(|se| se.is_transaction_canceled_exception()) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    // Still refused after re-reading: a newer copy is stored.
    let current = current_log(&st, &pk, &log).await?;
    Err(ApiError::Conflict(serde_json::to_value(current).unwrap_or(Value::Null)))
}
