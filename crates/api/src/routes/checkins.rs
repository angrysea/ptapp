//! Weekly check-ins (FB-3; ptcore::checkin): the coach's questions, and a
//! client's weekly answers.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::checkin::{answerable, default_template, week_of, Checkin, Template};
use ptcore::model::Role;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::parse_date;
use crate::AppState;

/// How many past weeks the check-in list returns.
const WEEKS_SHOWN: usize = 8;

/// A coach's questions, or the default ones.
pub async fn template_of(st: &AppState, coach: &str) -> ApiResult<Template> {
    Ok(st.store.get(&format!("COACH#{coach}"), "CHECKIN_TEMPLATE").await?.unwrap_or_else(default_template))
}

/// `GET /checkin-template`: the coach's own questions, or a client's coach's.
pub async fn get_template(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Template>> {
    let coach = match ctx.profile.role {
        Role::Coach | Role::Client => ctx.profile.coach_id.clone(),
        Role::Admin => return Err(ApiError::Forbidden("coaches and clients only")),
    };
    Ok(Json(template_of(&st, &coach).await?))
}

/// `PUT /checkin-template`.
pub async fn put_template(State(st): State<AppState>, ctx: Ctx, Json(mut t): Json<Template>) -> ApiResult<Json<Template>> {
    let coach = ctx.require_coach()?;
    for q in &mut t.questions {
        q.label = q.label.trim().to_string();
        q.unit = q.unit.take().map(|u| u.trim().to_string()).filter(|u| !u.is_empty());
    }
    t.validate().map_err(bad_request)?;
    st.store.put(&format!("COACH#{coach}"), "CHECKIN_TEMPLATE", &t).await?;
    Ok(Json(t))
}

/// `GET /clients/{cid}/checkins`: this week's Monday, the questions to answer
/// now, and the latest check-ins, newest first.
pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Value>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let mut all: Vec<Checkin> = st.store.query_prefix(&scope.pk(), "CHECKIN#").await?;
    all.sort_by_key(|c| std::cmp::Reverse(c.week_start));
    all.truncate(WEEKS_SHOWN);
    let template = template_of(&st, &scope.client.coach_id).await?;
    Ok(Json(json!({ "thisWeek": week_of(scope.today()), "template": template, "checkins": all })))
}

#[derive(Deserialize)]
pub struct Answers {
    answers: BTreeMap<String, Value>,
}

/// `PUT /clients/{cid}/checkins/{weekStart}`: the client answers (or edits)
/// this week's check-in, or last week's if they're late. The coach is told
/// the first time.
pub async fn put(State(st): State<AppState>, ctx: Ctx, Path((cid, week)): Path<(String, String)>, Json(body): Json<Answers>) -> ApiResult<Json<Checkin>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    if scope.by_coach {
        return Err(ApiError::Forbidden("only the client answers their check-in"));
    }
    let week_start = parse_date(&week)?;
    if !answerable(week_start, scope.today()) {
        return Err(bad_request("check-ins are for this week or last week, starting on a Monday"));
    }
    let sk = format!("CHECKIN#{week_start}");
    let existing: Option<Checkin> = st.store.get(&scope.pk(), &sk).await?;
    // An edit keeps the questions it first answered.
    let template = match &existing {
        Some(c) => Template { questions: c.questions.clone() },
        None => template_of(&st, &scope.client.coach_id).await?,
    };
    let answers = template.clean_answers(body.answers).map_err(bad_request)?;
    let checkin = Checkin { week_start, questions: template.questions, answers, submitted_at: Utc::now() };
    st.store.put(&scope.pk(), &sk, &checkin).await?;
    if existing.is_none() {
        let url = format!("#/c/{}/checkins", scope.client.user_id);
        super::notify::notify_user(&st, &scope.client.coach_id, &scope.client.name, "Sent their weekly check-in", &url, &format!("checkin-{}", scope.client.user_id)).await;
    }
    Ok(Json(checkin))
}
