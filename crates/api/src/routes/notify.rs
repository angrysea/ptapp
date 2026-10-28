//! Push notifications (MS-2): each device that turns them on stores its Web
//! Push subscription as `USER#{user}` / `PUSH#{endpoint}`; a new message or
//! video reply notifies every device of the recipient (routes/messages.rs).
//! Sending is push.rs; the service worker shows the notification (web/sw.js).
//!
//! A browser has one subscription whoever is signed in, so an endpoint
//! belongs to exactly one user: `PUSHEP#{endpoint}` / `OWNER` names them,
//! and subscribing takes it from anyone else (a shared device must not get
//! the previous user's notifications). Each user keeps at most
//! `MAX_DEVICES` subscriptions, oldest dropped first.

use std::time::Duration;

use axum::extract::State;
use axum::Json;
use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt};
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

/// Most devices a user gets notifications on.
pub const MAX_DEVICES: usize = 10;
/// Sends at once per notification, and how long each may take: sending
/// happens inside the request (a Lambda freezes work left after it answers).
const PARALLEL_SENDS: usize = 5;
const SEND_TIMEOUT: Duration = Duration::from_secs(3);

fn sk(endpoint: &str) -> String {
    format!("PUSH#{endpoint}")
}

fn owner_pk(endpoint: &str) -> String {
    format!("PUSHEP#{endpoint}")
}

#[derive(Serialize, Deserialize)]
struct Owner {
    user: String,
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
    let me = ctx.user_id().to_string();
    // The endpoint is this user's now: the previous owner (an earlier user of
    // this browser) stops getting notifications on it.
    // Only the browser holding the subscription can hand it over: it sends
    // the same keys. Anyone else who learned the endpoint is refused.
    if let Some(prev) = st.store.get::<Owner>(&owner_pk(&sub.endpoint), "OWNER").await?
        && prev.user != me
    {
        let theirs: Option<Subscription> = st.store.get(&format!("USER#{}", prev.user), &sk(&sub.endpoint)).await?;
        if let Some(theirs) = theirs {
            if theirs.keys.p256dh != sub.keys.p256dh || theirs.keys.auth != sub.keys.auth {
                return Err(bad_request("this device's notifications belong to another account; turn them off and on again"));
            }
            st.store.delete(&format!("USER#{}", prev.user), &sk(&sub.endpoint)).await?;
        }
    }
    st.store.put(&owner_pk(&sub.endpoint), "OWNER", &Owner { user: me.clone() }).await?;
    sub.created_at = Utc::now();
    let pk = format!("USER#{me}");
    st.store.put(&pk, &sk(&sub.endpoint), &sub).await?;
    // At most MAX_DEVICES: the oldest go.
    let mut all: Vec<Subscription> = st.store.query_prefix(&pk, "PUSH#").await?;
    if all.len() > MAX_DEVICES {
        all.sort_by_key(|s| s.created_at);
        for old in &all[..all.len() - MAX_DEVICES] {
            forget(&st, &me, &old.endpoint).await?;
        }
    }
    Ok(Json(json!({ "ok": true })))
}

/// Drops a user's subscription and, if they own it, the endpoint's owner record.
async fn forget(st: &AppState, user: &str, endpoint: &str) -> ApiResult<()> {
    st.store.delete(&format!("USER#{user}"), &sk(endpoint)).await?;
    if st.store.get::<Owner>(&owner_pk(endpoint), "OWNER").await?.is_some_and(|o| o.user == user) {
        st.store.delete(&owner_pk(endpoint), "OWNER").await?;
    }
    Ok(())
}

#[derive(Deserialize)]
pub struct Unsubscribe {
    endpoint: String,
}

/// `DELETE /me/push`: this device no longer wants notifications (or is signing out).
pub async fn unsubscribe(State(st): State<AppState>, ctx: Ctx, Json(body): Json<Unsubscribe>) -> ApiResult<Json<Value>> {
    forget(&st, ctx.user_id(), &body.endpoint).await?;
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
    // Each send owns its data, so the stream can be sent between threads.
    let results: Vec<_> = stream::iter(subs.into_iter().take(MAX_DEVICES))
        .map(|s| {
            let push = push.clone();
            let body = payload(sender, text, s.preview, url, tag);
            async move {
                let sub = json!({ "endpoint": s.endpoint, "keys": { "p256dh": s.keys.p256dh, "auth": s.keys.auth } });
                let sent = tokio::time::timeout(SEND_TIMEOUT, push.send(&sub, &body)).await;
                (s, sent.unwrap_or_else(|_| Err("timed out".into())))
            }
        })
        .buffer_unordered(PARALLEL_SENDS)
        .collect()
        .await;
    for (s, res) in results {
        match res {
            Ok(Sent::Delivered) => {}
            Ok(Sent::Gone) => {
                let _ = forget(st, user, &s.endpoint).await;
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
