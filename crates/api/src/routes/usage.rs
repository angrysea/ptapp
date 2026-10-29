//! Coach time per client (the Phase 2 exit gate: a coach handles 30+
//! clients without the app being the bottleneck). The coach's app counts
//! active seconds on each client's pages and posts them about once a minute;
//! they add up per week (Monday, coach's time zone) at
//! `COACH#{coach}` / `USAGE#{weekStart}#{clientId}`.

use std::collections::HashSet;

use aws_sdk_dynamodb::types::AttributeValue;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{Duration, Utc};
use ptcore::checkin::week_of;
use ptcore::model::{Profile, RosterEntry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::store::{key, s};
use crate::AppState;

/// More than this in one post isn't plausible (posts come every minute or so).
const MAX_SECONDS: u32 = 900;
const HIGH: &str = "~";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    client_id: String,
    seconds: u32,
}

#[derive(Deserialize)]
pub struct Batch {
    entries: Vec<Entry>,
}

/// `POST /usage`: active seconds per client since the last post.
pub async fn record(State(st): State<AppState>, ctx: Ctx, Json(body): Json<Batch>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    if body.entries.len() > 50 {
        return Err(bad_request("too many entries"));
    }
    // Checked before anything is added, so a refused post adds nothing.
    if body.entries.iter().any(|e| e.seconds > MAX_SECONDS) {
        return Err(bad_request(format!("at most {MAX_SECONDS} seconds per post")));
    }
    let week = week_of(ctx.profile.today(Utc::now()));
    let pk = format!("COACH#{coach}");
    let roster: HashSet<String> = st.store.query_prefix::<RosterEntry>(&pk, "CLIENT#").await?.into_iter().map(|r| r.client_id).collect();
    let mut added = 0;
    for e in body.entries.iter().filter(|e| e.seconds > 0) {
        // Only this coach's own clients count.
        if !roster.contains(&e.client_id) {
            continue;
        }
        st.store
            .ddb
            .update_item()
            .table_name(&st.store.table)
            .set_key(Some(key(&pk, &format!("USAGE#{week}#{}", e.client_id))))
            .update_expression("SET clientId = :c, weekStart = :w ADD seconds :s")
            .expression_attribute_values(":c", s(&e.client_id))
            .expression_attribute_values(":w", s(week.to_string()))
            .expression_attribute_values(":s", AttributeValue::N(e.seconds.to_string()))
            .send()
            .await?;
        added += e.seconds;
    }
    Ok(Json(json!({ "added": added })))
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    client_id: String,
    week_start: String,
    seconds: u64,
}

#[derive(Deserialize)]
pub struct WeeksQuery {
    weeks: Option<i64>,
}

/// A coach's time over the last `weeks` weeks: per week, and the average
/// minutes per client per week (over the clients they have now).
async fn summary(st: &AppState, coach: &Profile, weeks: i64) -> ApiResult<Value> {
    let pk = format!("COACH#{}", coach.user_id);
    let this_week = week_of(coach.today(Utc::now()));
    let from = this_week - Duration::weeks(weeks - 1);
    let rows: Vec<Usage> = st.store.query_between(&pk, &format!("USAGE#{from}"), &format!("USAGE#{HIGH}"), true).await?;
    let clients = st.store.query_prefix::<RosterEntry>(&pk, "CLIENT#").await?.len();
    let per_week: Vec<Value> = (0..weeks)
        .map(|i| {
            let w = (from + Duration::weeks(i)).to_string();
            let these: Vec<&Usage> = rows.iter().filter(|r| r.week_start == w).collect();
            json!({ "weekStart": w, "minutes": these.iter().map(|r| r.seconds).sum::<u64>() / 60, "clientsSeen": these.len() })
        })
        .collect();
    let total_minutes = rows.iter().map(|r| r.seconds).sum::<u64>() as f64 / 60.0;
    let avg = (clients > 0).then(|| (total_minutes / clients as f64 / weeks as f64 * 10.0).round() / 10.0);
    Ok(json!({ "weeks": per_week, "clients": clients, "avgMinutesPerClientPerWeek": avg }))
}

/// `GET /usage?weeks=4`: the coach's own.
pub async fn mine(State(st): State<AppState>, ctx: Ctx, Query(q): Query<WeeksQuery>) -> ApiResult<Json<Value>> {
    ctx.require_coach()?;
    Ok(Json(summary(&st, &ctx.profile, q.weeks.unwrap_or(4).clamp(1, 26)).await?))
}

/// `GET /admin/coaches/{id}/usage?weeks=4`.
pub async fn for_admin(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Query(q): Query<WeeksQuery>) -> ApiResult<Json<Value>> {
    ctx.require_admin()?;
    let coach: Profile = st.store.get(&format!("USER#{id}"), "PROFILE").await?.ok_or(ApiError::NotFound("coach not found"))?;
    Ok(Json(summary(&st, &coach, q.weeks.unwrap_or(4).clamp(1, 26)).await?))
}
