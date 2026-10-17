//! WB-2..WB-4: workouts and multi-week programs, stored as one document each.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::Program;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Ctx;
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
        for b in &mut pw.workout.blocks {
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
    p.workouts.sort_by_key(|pw| (pw.week, pw.day));
    p.updated_at = Some(Utc::now());
}

pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Program>>> {
    let coach = ctx.require_coach()?;
    let mut programs: Vec<Program> = st.store.query_prefix(&pk(coach), "PROGRAM#").await?;
    programs.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
    Ok(Json(programs))
}

pub async fn get_one(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Program>> {
    let coach = ctx.require_coach()?;
    st.store.get(&pk(coach), &sk(&id)).await?.map(Json).ok_or(ApiError::NotFound("program"))
}

pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(mut p): Json<Program>) -> ApiResult<Json<Program>> {
    let coach = ctx.require_coach()?;
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
) -> ApiResult<Json<Program>> {
    let coach = ctx.require_coach()?;
    let _existing: Program = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("program"))?;
    p.id = id;
    normalize(&mut p);
    p.validate().map_err(bad_request)?;
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(Json(p))
}

pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    st.store.delete(&pk(coach), &sk(&id)).await?;
    Ok(Json(json!({ "ok": true })))
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
    let coach = ctx.require_coach()?;
    let mut p: Program = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("program"))?;
    p.id = new_id();
    p.name = body.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} (copy)", p.name));
    for pw in &mut p.workouts {
        pw.workout.id.clear();
        for b in &mut pw.workout.blocks {
            b.id.clear();
            for it in &mut b.items {
                it.id.clear();
            }
        }
    }
    normalize(&mut p);
    st.store.put(&pk(coach), &sk(&p.id), &p).await?;
    Ok(Json(p))
}
