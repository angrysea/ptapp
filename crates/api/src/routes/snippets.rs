//! Saved reply snippets (FB-4): a coach's reusable bits of feedback, picked
//! from any message box or typed with a `/` shortcut. `{name}` in a snippet
//! becomes the client's first name when it's inserted (in the app).

use axum::extract::{Path, State};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::util::new_id;
use crate::AppState;

const MAX_TITLE: usize = 60;
const MAX_TEXT: usize = 2000;
const MAX_SNIPPETS: usize = 200;

/// `COACH#{coach}` / `SNIPPET#{id}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snippet {
    pub id: String,
    pub title: String,
    pub text: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize)]
pub struct SnippetIn {
    title: String,
    text: String,
}

impl SnippetIn {
    fn clean(self) -> ApiResult<(String, String)> {
        let title = self.title.trim().to_string();
        let text = self.text.trim().to_string();
        if title.is_empty() || title.chars().count() > MAX_TITLE {
            return Err(bad_request(format!("a snippet's name must be 1-{MAX_TITLE} characters")));
        }
        if text.is_empty() || text.chars().count() > MAX_TEXT {
            return Err(bad_request(format!("a snippet's text must be 1-{MAX_TEXT} characters")));
        }
        Ok((title, text))
    }
}

fn pk(coach: &str) -> String {
    format!("COACH#{coach}")
}

fn sk(id: &str) -> String {
    format!("SNIPPET#{id}")
}

/// `GET /snippets`, by name.
pub async fn list(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Vec<Snippet>>> {
    let coach = ctx.require_coach()?;
    let mut all: Vec<Snippet> = st.store.query_prefix(&pk(coach), "SNIPPET#").await?;
    all.sort_by_key(|s| s.title.to_lowercase());
    Ok(Json(all))
}

pub async fn create(State(st): State<AppState>, ctx: Ctx, Json(body): Json<SnippetIn>) -> ApiResult<Json<Snippet>> {
    let coach = ctx.require_coach()?;
    let (title, text) = body.clean()?;
    let existing: Vec<Snippet> = st.store.query_prefix(&pk(coach), "SNIPPET#").await?;
    if existing.len() >= MAX_SNIPPETS {
        return Err(bad_request(format!("you can keep up to {MAX_SNIPPETS} snippets; delete some first")));
    }
    let s = Snippet { id: new_id(), title, text, updated_at: Utc::now() };
    st.store.put(&pk(coach), &sk(&s.id), &s).await?;
    Ok(Json(s))
}

pub async fn update(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>, Json(body): Json<SnippetIn>) -> ApiResult<Json<Snippet>> {
    let coach = ctx.require_coach()?;
    let mut s: Snippet = st.store.get(&pk(coach), &sk(&id)).await?.ok_or(ApiError::NotFound("snippet"))?;
    (s.title, s.text) = body.clean()?;
    s.updated_at = Utc::now();
    st.store.put(&pk(coach), &sk(&id), &s).await?;
    Ok(Json(s))
}

pub async fn remove(State(st): State<AppState>, ctx: Ctx, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let coach = ctx.require_coach()?;
    if st.store.get::<Snippet>(&pk(coach), &sk(&id)).await?.is_none() {
        return Err(ApiError::NotFound("snippet"));
    }
    st.store.delete(&pk(coach), &sk(&id)).await?;
    Ok(Json(json!({ "ok": true })))
}
