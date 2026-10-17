//! FB-1: comments on a whole workout or a single exercise/set; also serves
//! as a simple async message thread until real-time chat ships.

use axum::extract::{Path, State};
use axum::Json;
use chrono::Utc;
use ptcore::model::Comment;
use serde::Deserialize;

use crate::auth::{client_scope, Ctx};
use crate::error::{bad_request, ApiResult};
use crate::util::new_id;
use crate::AppState;

pub async fn list(State(st): State<AppState>, ctx: Ctx, Path(cid): Path<String>) -> ApiResult<Json<Vec<Comment>>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    Ok(Json(st.store.query_prefix(&scope.pk(), "COMMENT#").await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewComment {
    text: String,
    log_id: Option<String>,
    item_id: Option<String>,
    set_index: Option<u32>,
}

pub async fn create(
    State(st): State<AppState>,
    ctx: Ctx,
    Path(cid): Path<String>,
    Json(body): Json<NewComment>,
) -> ApiResult<Json<Comment>> {
    let scope = client_scope(&st, &ctx, &cid).await?;
    let text = body.text.trim();
    if text.is_empty() || text.len() > 4000 {
        return Err(bad_request("comment must be 1-4000 characters"));
    }
    let c = Comment {
        id: new_id(),
        author_id: ctx.user_id().to_string(),
        author_role: ctx.profile.role,
        author_name: ctx.profile.name.clone(),
        log_id: body.log_id,
        item_id: body.item_id,
        set_index: body.set_index,
        text: text.to_string(),
        created_at: Utc::now(),
    };
    st.store
        .put(&scope.pk(), &format!("COMMENT#{}#{}", c.created_at.to_rfc3339(), c.id), &c)
        .await?;
    Ok(Json(c))
}
