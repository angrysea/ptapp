//! A coach's reminders and automated messages (MS-4; ptcore::automation),
//! which the hourly sweep (sweep.rs) sends.

use axum::extract::State;
use axum::Json;
use ptcore::automation::Automation;

use crate::auth::Ctx;
use crate::error::{bad_request, ApiResult};
use crate::AppState;

/// `GET /automation`: the coach's settings (defaults until they save some).
pub async fn get(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Automation>> {
    let coach = ctx.require_coach()?;
    Ok(Json(st.store.get(&format!("COACH#{coach}"), "AUTOMATION").await?.unwrap_or_default()))
}

/// `PUT /automation`.
pub async fn put(State(st): State<AppState>, ctx: Ctx, Json(mut a): Json<Automation>) -> ApiResult<Json<Automation>> {
    let coach = ctx.require_coach()?;
    a.validate().map_err(bad_request)?;
    st.store.put(&format!("COACH#{coach}"), "AUTOMATION", &a).await?;
    Ok(Json(a))
}
