//! Push notifications (MS-2): each device that turns them on stores its Web
//! Push subscription as `USER#{user}` / `PUSH#{endpoint}`; a new message or
//! video reply notifies every device of the recipient (routes/messages.rs).
//! Sending is push.rs; the service worker shows the notification (web/sw.js).

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth::Ctx;
use crate::error::{bad_request, ApiResult};
use crate::push::{is_push_service, Sent};
use crate::AppState;

/// A browser's push subscription (`PushSubscription.toJSON()`), plus the
/// user's choice of whether its notifications show the message text.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Subscription {
    pub endpoint: String,
    pub keys: SubscriptionKeys,
    #[serde(default = "yes")]
    pub preview: bool,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

fn yes() -> bool {
    true
}

fn sk(endpoint: &str) -> String {
    format!("PUSH#{endpoint}")
}

/// `GET /push/key`: the VAPID public key the browser subscribes with; null
/// when push isn't set up (local development).
pub async fn public_key(State(st): State<AppState>, _ctx: Ctx) -> Json<Value> {
    Json(json!({ "publicKey": st.push.as_ref().map(|p| p.public_key.clone()) }))
}

/// `POST /me/push`: this device wants notifications (again; it's idempotent).
pub async fn subscribe(State(st): State<AppState>, ctx: Ctx, Json(mut sub): Json<Subscription>) -> ApiResult<Json<Value>> {
    if !is_push_service(&sub.endpoint) || sub.endpoint.len() > 900 {
        return Err(bad_request("that isn't a browser push subscription"));
    }
    if sub.keys.p256dh.is_empty() || sub.keys.auth.is_empty() || sub.keys.p256dh.len() > 200 || sub.keys.auth.len() > 100 {
        return Err(bad_request("the subscription's keys are missing"));
    }
    sub.created_at = Utc::now();
    st.store.put(&format!("USER#{}", ctx.user_id()), &sk(&sub.endpoint), &sub).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct Unsubscribe {
    endpoint: String,
}

/// `DELETE /me/push`: this device no longer wants notifications (or is signing out).
pub async fn unsubscribe(State(st): State<AppState>, ctx: Ctx, Json(body): Json<Unsubscribe>) -> ApiResult<Json<Value>> {
    st.store.delete(&format!("USER#{}", ctx.user_id()), &sk(&body.endpoint)).await?;
    Ok(Json(json!({ "ok": true })))
}

/// What a notification says. Without a preview it only names the sender.
pub fn payload(sender: &str, text: &str, preview: bool, url: &str, tag: &str) -> Value {
    let body = if preview {
        let short: String = text.chars().take(120).collect();
        if short.len() < text.len() { format!("{short}…") } else { short }
    } else {
        "New message".to_string()
    };
    json!({ "title": sender, "body": body, "url": url, "tag": tag })
}

/// Notifies every device of `user`. Best effort: failures are logged, and
/// subscriptions the push service says are gone are deleted.
pub async fn notify_user(st: &AppState, user: &str, sender: &str, text: &str, url: &str, tag: &str) {
    let Some(push) = &st.push else { return };
    let pk = format!("USER#{user}");
    let subs: Vec<Subscription> = match st.store.query_prefix(&pk, "PUSH#").await {
        Ok(s) => s,
        Err(e) => return tracing::warn!(user, error = ?e, "couldn't read push subscriptions"),
    };
    let sends = subs.iter().map(|s| async move {
        let sub = json!({ "endpoint": s.endpoint, "keys": { "p256dh": s.keys.p256dh, "auth": s.keys.auth } });
        (s, push.send(&sub, &payload(sender, text, s.preview, url, tag)).await)
    });
    for (s, res) in join_all(sends).await {
        match res {
            Ok(Sent::Delivered) => {}
            Ok(Sent::Gone) => {
                let _ = st.store.delete(&pk, &sk(&s.endpoint)).await;
            }
            Err(e) => tracing::warn!(user, error = %e, "push failed"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::payload;

    #[test]
    fn notifications_show_the_text_only_with_previews_on() {
        let p = payload("Coach Jo", "Knees out at the bottom", true, "#/coach", "thread-c1");
        assert_eq!((p["title"].as_str(), p["body"].as_str()), (Some("Coach Jo"), Some("Knees out at the bottom")));
        let p = payload("Coach Jo", "Knees out at the bottom", false, "#/coach", "thread-c1");
        assert_eq!(p["body"], "New message");
        let long = "a".repeat(200);
        assert_eq!(payload("x", &long, true, "u", "t")["body"].as_str().unwrap().chars().count(), 121);
    }
}
