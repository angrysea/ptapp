//! Chat transport spike (tasks/chat/todo.md, Tasks 1-2): lets the spike page
//! check real-time delivery and Web Push end to end on dev. Each only reaches
//! the caller. Removed once the messages API (Task 3) and push sending
//! (Task 5) replace them; `public_key` stays.

use axum::extract::State;
use axum::Json;
use chrono::Utc;
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiResult};
use crate::AppState;

/// The VAPID public key the browser subscribes with; null when push isn't set up.
pub async fn public_key(State(st): State<AppState>, _ctx: Ctx) -> Json<Value> {
    Json(json!({ "publicKey": st.push.as_ref().map(|p| p.public_key.clone()) }))
}

/// Publishes a ping to the caller's own inbox channel.
pub async fn ping(State(st): State<AppState>, ctx: Ctx) -> ApiResult<Json<Value>> {
    let Some(rt) = &st.realtime else { return Err(bad_request("real-time isn't configured")) };
    let at = Utc::now();
    rt.notify(ctx.user_id(), &json!({ "type": "ping", "at": at })).await;
    Ok(Json(json!({ "published": true, "at": at })))
}

/// Sends a test notification to the subscription in the body (the caller's own browser).
pub async fn push(State(st): State<AppState>, ctx: Ctx, Json(sub): Json<Value>) -> ApiResult<Json<Value>> {
    let Some(push) = &st.push else { return Err(bad_request("push isn't configured")) };
    let payload = json!({ "title": "PT Coach", "body": format!("Test notification for {}", ctx.profile.name), "url": "/" });
    match push.send(&sub, &payload).await {
        Ok(sent) => Ok(Json(json!({ "result": format!("{sent:?}") }))),
        // Our own short message (push.rs logs the service's reply), never an upstream body.
        Err(e) => Ok(Json(json!({ "result": "Error", "error": e }))),
    }
}
