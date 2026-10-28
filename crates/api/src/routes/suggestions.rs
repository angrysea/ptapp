//! Progression suggestions (WB-6; ptcore::progression). Saving a log checks
//! the progression rules of the workout it was for; each rule met suggests
//! a change to the next workout with that exercise. The coach accepts
//! (the change is written into that workout) or dismisses it.

use axum::extract::{Path, State};
use axum::Json;
use chrono::{Duration, Utc};
use ptcore::model::{Item, ScheduledWorkout, WorkoutLog};
use ptcore::progression::{earned, propose, Change, Suggestion, SuggestionStatus};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, ClientScope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::AppState;

const HIGH: &str = "~";

fn sk(id: &str) -> String {
    format!("SUGGEST#{id}")
}

fn sched_sk(sw: &ScheduledWorkout) -> String {
    format!("SCHED#{}#{}", sw.date, sw.id)
}

fn current_of(item: &Item) -> Change {
    Change { load_kg: item.load_kg, reps: Some(item.reps.clone()) }
}

/// Checks `log` against its workout's progression rules and files a
/// suggestion for each one met. Called after the log is saved.
pub async fn evaluate(st: &AppState, scope: &ClientScope, log: &WorkoutLog) -> ApiResult<usize> {
    let Some(sched_id) = log.scheduled_id.as_deref() else { return Ok(0) };
    if !log.status.counts_as_done() {
        return Ok(0);
    }
    let pk = scope.pk();
    // The workout this log was for (logged within two weeks of its date)
    // and everything after it.
    let from = log.date - Duration::days(14);
    let sched: Vec<ScheduledWorkout> = st.store.query_between(&pk, &format!("SCHED#{from}"), &format!("SCHED#{HIGH}"), true).await?;
    let Some(this) = sched.iter().find(|s| s.id == sched_id) else { return Ok(0) };
    let rules: Vec<&Item> = this.workout.blocks.iter().flat_map(|b| &b.items).filter(|i| i.progression.is_some()).collect();
    if rules.is_empty() {
        return Ok(0);
    }
    let logged: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{}", this.date), &format!("LOG#{HIGH}"), true).await?;
    let is_logged = |s: &ScheduledWorkout| logged.iter().any(|l| l.scheduled_id.as_deref() == Some(s.id.as_str()));
    let coach_pk = format!("COACH#{}", scope.client.coach_id);
    let mut filed = 0;
    for item in rules {
        let Some(won) = earned(item, log) else { continue };
        let rule = item.progression.as_ref().expect("filtered on progression");
        // The next workout with this exercise that hasn't been done yet.
        let next = sched.iter().filter(|s| s.date > this.date && s.id != this.id && !is_logged(s)).find_map(|s| {
            s.workout.blocks.iter().flat_map(|b| &b.items).find(|i| i.exercise_id == item.exercise_id).map(|i| (s, i))
        });
        let Some((target, target_item)) = next else { continue };
        let Some(proposed) = propose(rule, &won, target_item) else { continue };
        let suggestion = Suggestion {
            id: format!("{}.{}", log.id, item.id),
            client_id: scope.client.user_id.clone(),
            client_name: scope.client.name.clone(),
            exercise_id: item.exercise_id.clone(),
            exercise_name: item.exercise_name.clone(),
            log_id: log.id.clone(),
            log_date: log.date,
            why: won.why,
            target_sched_id: target.id.clone(),
            target_date: target.date,
            target_item_id: target_item.id.clone(),
            current: current_of(target_item),
            proposed,
            status: SuggestionStatus::Pending,
            created_at: Utc::now(),
            decided_at: None,
        };
        if st.store.put_new(&coach_pk, &sk(&suggestion.id), &suggestion).await? {
            filed += 1;
        }
    }
    Ok(filed)
}

/// `GET /suggestions`: the coach's pending suggestions, oldest first.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Suggestion>>> {
    let coach = ctx.require_coach()?;
    let mut all: Vec<Suggestion> = st.store.query_prefix(&format!("COACH#{coach}"), "SUGGEST#").await?;
    all.retain(|s| s.status == SuggestionStatus::Pending);
    all.sort_by_key(|s| s.created_at);
    Ok(Json(all))
}

async fn load(st: &AppState, coach: &str, id: &str) -> ApiResult<Suggestion> {
    let s: Suggestion = st.store.get(&format!("COACH#{coach}"), &sk(id)).await?.ok_or(ApiError::NotFound("suggestion"))?;
    if s.status != SuggestionStatus::Pending {
        return Err(bad_request("this suggestion was already handled"));
    }
    Ok(s)
}

async fn decide(st: &AppState, coach: &str, mut s: Suggestion, status: SuggestionStatus) -> ApiResult<Suggestion> {
    s.status = status;
    s.decided_at = Some(Utc::now());
    st.store.put(&format!("COACH#{coach}"), &sk(&s.id), &s).await?;
    Ok(s)
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Accept {
    /// The coach's own numbers instead of the suggested ones.
    #[serde(default)]
    load_kg: Option<f64>,
    #[serde(default)]
    reps: Option<String>,
}

/// `POST /suggestions/{id}/accept`: writes the change into the target
/// workout, unless the client has already done it.
pub async fn accept(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, body: Option<Json<Accept>>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?.to_string();
    let s = load(&st, &coach, &id).await?;
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let load_kg = body.load_kg.or(s.proposed.load_kg);
    if load_kg.is_some_and(|k| !(0.0..1000.0).contains(&k)) {
        return Err(bad_request("load must be 0-1000 kg"));
    }
    let reps = body.reps.map(|r| r.trim().to_string()).filter(|r| !r.is_empty()).or(s.proposed.reps.clone());
    if reps.as_ref().is_some_and(|r| r.chars().count() > 20) {
        return Err(bad_request("reps must be at most 20 characters"));
    }
    // Still this coach's client? (They may have left since the suggestion was filed.)
    let scope = match client_scope(&st, &ctx, &s.client_id).await {
        Ok(scope) => scope,
        Err(e) => {
            decide(&st, &coach, s, SuggestionStatus::Expired).await?;
            return Err(e);
        }
    };
    let pk = scope.pk();
    let sk_target = format!("SCHED#{}#{}", s.target_date, s.target_sched_id);
    let Some(mut target) = st.store.get::<ScheduledWorkout>(&pk, &sk_target).await? else {
        decide(&st, &coach, s, SuggestionStatus::Expired).await?;
        return Err(ApiError::Conflict(json!({ "message": "that workout is no longer on the client's calendar" })));
    };
    let logged: Vec<WorkoutLog> = st.store.query_between(&pk, &format!("LOG#{}", s.target_date - Duration::days(14)), &format!("LOG#{HIGH}"), true).await?;
    if logged.iter().any(|l| l.scheduled_id.as_deref() == Some(target.id.as_str())) {
        decide(&st, &coach, s, SuggestionStatus::Expired).await?;
        return Err(ApiError::Conflict(json!({ "message": "the client has already done that workout" })));
    }
    let Some(item) = target.workout.blocks.iter_mut().flat_map(|b| b.items.iter_mut()).find(|i| i.id == s.target_item_id) else {
        decide(&st, &coach, s, SuggestionStatus::Expired).await?;
        return Err(ApiError::Conflict(json!({ "message": "that exercise is no longer in the workout" })));
    };
    if let Some(kg) = load_kg {
        item.load_kg = Some(kg);
        item.load_pct = None; // a set weight now, so a new max doesn't re-weigh it
    }
    if let Some(r) = reps {
        item.reps = r;
    }
    st.store.put(&pk, &sched_sk(&target), &target).await?;
    let s = decide(&st, &coach, s, SuggestionStatus::Accepted).await?;
    Ok(Json(json!({ "suggestion": s, "workout": target })))
}

/// `POST /suggestions/{id}/dismiss`.
pub async fn dismiss(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Suggestion>> {
    let coach = ctx.require_coach()?.to_string();
    let s = load(&st, &coach, &id).await?;
    Ok(Json(decide(&st, &coach, s, SuggestionStatus::Dismissed).await?))
}
