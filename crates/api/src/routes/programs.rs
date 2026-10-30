//! WB-2..WB-4: workouts and multi-week programs, stored as one document each.
//! A coach's programs are templates, or copies made for one client
//! (`client_id`), which assigning creates so each client's training can be
//! personalized; editing a client's program re-writes their upcoming
//! workouts from it (`resync`).

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use ptcore::cardio::add_program_defaults;
use ptcore::model::{Assignment, Block, Profile, Program, ScheduledWorkout, WorkoutLog};
use ptcore::schedule::{end_date, materialize};
use ptcore::starters::starter_programs;
use ptcore::strength::resolve_workout;
use serde::Deserialize;
use serde_json::{json, Value};

use ptcore::team::Perm;
use crate::auth::{client_scope_as, ClientScope, Ctx};
use crate::routes::maxes::maxes_by_exercise;
use crate::store::keyed;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::new_id;
use crate::AppState;

fn pk(coach: &str) -> String {
    format!("COACH#{coach}")
}

fn sk(id: &str) -> String {
    format!("PROGRAM#{id}")
}

/// Fill in any ids the client left empty so every workout/block/item is addressable.
fn normalize(p: &mut Program) {
    for pw in &mut p.workouts {
        if pw.workout.id.is_empty() {
            pw.workout.id = new_id();
        }
        fill_ids(&mut pw.workout.blocks);
    }
    fill_ids(&mut p.default_blocks);
    p.workouts.sort_by_key(|pw| (pw.week, pw.day));
    p.updated_at = Some(Utc::now());
}

fn fill_ids(blocks: &mut [Block]) {
    for b in blocks {
        if b.id.is_empty() {
            b.id = new_id();
        }
        for it in &mut b.items {
            if it.id.is_empty() {
                it.id = new_id();
            }
        }
    }
}

fn clear_ids(blocks: &mut [Block]) {
    for b in blocks {
        b.id.clear();
        for it in &mut b.items {
            it.id.clear();
        }
    }
}

/// Whether the caller may see this program: templates are the whole team's;
/// a client's program, only the head coach's and that client's assistants'.
fn visible(ctx: &Ctx, p: &Program) -> bool {
    p.client_id.as_deref().is_none_or(|c| ctx.sees_client(c))
}

fn check_visible(ctx: &Ctx, p: &Program) -> ApiResult<()> {
    if visible(ctx, p) { Ok(()) } else { Err(ApiError::Forbidden("this program belongs to a client who isn't assigned to you")) }
}

pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Program>>> {
    let coach = ctx.require(Perm::Programming)?;
    let mut programs: Vec<Program> = st.store.query_prefix(&pk(coach), "PROGRAM#").await?;
    programs.retain(|p| visible(&ctx, p));
    programs.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
    Ok(Json(programs))
}

pub async fn get_one(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Program>> {
    let coach = ctx.require(Perm::Programming)?;
    let p: Program = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("program"))?;
    check_visible(&ctx, &p)?;
    Ok(Json(p))
}

pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(mut p): Json<Program>) -> ApiResult<Json<Program>> {
    let coach = ctx.require(Perm::Programming)?;
    // A program for one client must be for one of this coach's clients.
    p.client_name = match &p.client_id {
        Some(cid) => Some(client_scope_as(&st, &ctx, cid, &[Perm::Programming]).await?.client.name),
        None => None,
    };
    p.id = new_id();
    normalize(&mut p);
    p.validate().map_err(bad_request)?;
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(Json(p))
}

pub async fn update(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(mut p): Json<Program>,
) -> ApiResult<Json<Value>> {
    let coach = ctx.require(Perm::Programming)?;
    let existing: Program = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("program"))?;
    check_visible(&ctx, &existing)?;
    p.id = id;
    // Who a program is for is set when it's made, not by an edit.
    (p.client_id, p.client_name) = (existing.client_id, existing.client_name);
    normalize(&mut p);
    p.validate().map_err(bad_request)?;
    // A client's program: their upcoming workouts follow the edit. Checked
    // before saving, so a program that would no longer fit isn't saved.
    let scope = match &p.client_id {
        Some(cid) => client_scope_as(&st, &ctx, cid, &[Perm::Programming]).await.ok(),
        None => None,
    };
    let plan = match &scope {
        Some(scope) => Some(resync_plan(&st, scope, &p).await?),
        None => None,
    };
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    let updated = match (scope, plan) {
        (Some(scope), Some(plan)) => apply_resync(&st, &scope, &p, plan).await?,
        _ => 0,
    };
    let mut out = serde_json::to_value(&p).map_err(|e| ApiError::Internal(e.to_string()))?;
    out["updatedWorkouts"] = json!(updated);
    Ok(Json(out))
}

/// A copy of `source` made for `client`: their own program to personalize.
pub async fn copy_for_client(st: &AppState, coach: &str, mut p: Program, client: &Profile) -> ApiResult<Program> {
    p.id = new_id();
    p.client_id = Some(client.user_id.clone());
    p.client_name = Some(client.name.clone());
    p.is_template = false;
    for pw in &mut p.workouts {
        pw.workout.id.clear();
        clear_ids(&mut pw.workout.blocks);
    }
    clear_ids(&mut p.default_blocks);
    normalize(&mut p);
    p.validate().map_err(bad_request)?;
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(p)
}

/// What re-writing a client's upcoming workouts from their edited program
/// touches: per assignment of it, the unlogged workouts from tomorrow on to
/// replace (today's stays, as the client may be in it), and the dates
/// already logged, which keep their workout.
struct Resync {
    assignments: Vec<(Assignment, Vec<ScheduledWorkout>, HashSet<NaiveDate>)>,
}

async fn resync_plan(st: &AppState, scope: &ClientScope, p: &Program) -> ApiResult<Resync> {
    let cpk = scope.pk();
    let tomorrow = scope.today() + Duration::days(1);
    let all: Vec<Assignment> = st.store.query_prefix(&cpk, "ASSIGN#").await?;
    let upcoming: Vec<ScheduledWorkout> = st.store.query_between(&cpk, &format!("SCHED#{tomorrow}"), "SCHED#~", true).await?;
    let logged: Vec<WorkoutLog> = st.store.query_between(&cpk, &format!("LOG#{tomorrow}"), "LOG#~", true).await?;
    let logged_ids: HashSet<&str> = logged.iter().filter_map(|l| l.scheduled_id.as_deref()).collect();
    let mut out = vec![];
    for a in all.iter().filter(|a| a.program_id == p.id) {
        let end = end_date(a.start_date, p.weeks);
        if let Some(clash) = all.iter().find(|o| o.id != a.id && o.start_date <= end && a.start_date <= o.end_date) {
            return Err(bad_request(format!(
                "with {} weeks this program would run into the {} assignment that starts {}; shorten it or move that one first",
                p.weeks, clash.program_name, clash.start_date
            )));
        }
        let mine: Vec<ScheduledWorkout> = upcoming.iter().filter(|s| s.assignment_id == a.id).cloned().collect();
        let keep: HashSet<NaiveDate> = mine.iter().filter(|s| logged_ids.contains(s.id.as_str())).map(|s| s.date).collect();
        let replace = mine.into_iter().filter(|s| !logged_ids.contains(s.id.as_str())).collect();
        out.push((a.clone(), replace, keep));
    }
    Ok(Resync { assignments: out })
}

/// Re-writes the planned workouts from the saved program. Returns how many
/// upcoming workouts were written.
async fn apply_resync(st: &AppState, scope: &ClientScope, p: &Program, plan: Resync) -> ApiResult<usize> {
    let cpk = scope.pk();
    let tomorrow = scope.today() + Duration::days(1);
    let maxes = maxes_by_exercise(st, &cpk).await?;
    let mut written = 0;
    for (mut a, replace, keep) in plan.assignments {
        st.store.batch_delete(replace.iter().map(|s| (cpk.clone(), format!("SCHED#{}#{}", s.date, s.id))).collect()).await?;
        let mut items = vec![];
        for (date, idx) in materialize(p, a.start_date).into_iter().filter(|(d, _)| *d >= tomorrow && !keep.contains(d)) {
            let pw = &p.workouts[idx];
            let mut sw = ScheduledWorkout {
                id: new_id(),
                date,
                assignment_id: a.id.clone(),
                program_name: p.name.clone(),
                week: pw.week,
                day: pw.day,
                workout: pw.workout.clone(),
            };
            add_program_defaults(&mut sw.workout, &p.default_blocks);
            resolve_workout(&mut sw.workout, &maxes, scope.client.units);
            items.push(keyed(&cpk, &format!("SCHED#{date}#{}", sw.id), &sw)?);
        }
        written += items.len();
        (a.end_date, a.program_name) = (end_date(a.start_date, p.weeks), p.name.clone());
        items.push(keyed(&cpk, &format!("ASSIGN#{}", a.id), &a)?);
        st.store.batch_put(items).await?;
    }
    Ok(written)
}

pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require(Perm::Programming)?;
    if let Some(p) = st.store.get::<Program>(&pk(coach), &sk(&id)).await? {
        check_visible(&ctx, &p)?;
    }
    st.store.delete(&pk(coach), &sk(&id)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// `GET /programs/starters`: the ready-made programs (ptcore::starters)
/// a coach can start from.
pub async fn starters(ctx: Ctx) -> ApiResult<Json<Vec<Program>>> {
    ctx.require(Perm::Programming)?;
    Ok(Json(starter_programs()))
}

/// `POST /programs/starters/{key}/copy`: the coach's own copy of a starter
/// program, to edit and assign like any other.
pub async fn copy_starter(State(st): State<AppState>, ctx: Ctx, Path(key): Path<String>) -> ApiResult<Json<Program>> {
    let coach = ctx.require(Perm::Programming)?;
    let mut p = starter_programs().into_iter().find(|p| p.id == key).ok_or(ApiError::NotFound("starter program"))?;
    p.id = new_id();
    normalize(&mut p);
    p.validate().map_err(bad_request)?;
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(Json(p))
}

#[derive(Deserialize)]
pub struct CopyBody {
    name: Option<String>,
}

/// WB-4: copy a program (e.g. to reuse a template for another client).
pub async fn copy(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(id): Path<String>,
    Json(body): Json<CopyBody>,
) -> ApiResult<Json<Program>> {
    let coach = ctx.require(Perm::Programming)?;
    let mut p: Program = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("program"))?;
    check_visible(&ctx, &p)?;
    p.id = new_id();
    p.name = body.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} (copy)", p.name));
    for pw in &mut p.workouts {
        pw.workout.id.clear();
        clear_ids(&mut pw.workout.blocks);
    }
    clear_ids(&mut p.default_blocks);
    normalize(&mut p);
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(Json(p))
}
