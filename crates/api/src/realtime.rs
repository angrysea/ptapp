//! Real-time delivery (MS-1): publishes events to a user's AppSync Events
//! channel, `/inbox/{userId}`. Only that user may subscribe to it (the
//! channel namespace's onSubscribe handler in template.yaml) and only this
//! Lambda may publish (IAM). Best effort: the table is the source of truth,
//! and the app refetches when it reconnects or comes back to the foreground.

use std::time::SystemTime;

use aws_credential_types::provider::{ProvideCredentials, SharedCredentialsProvider};
use aws_sigv4::http_request::{sign, SignableBody, SignableRequest, SigningSettings};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Realtime {
    http: reqwest::Client,
    /// `https://{EVENTS_HTTP_DOMAIN}/event`
    url: String,
    region: String,
    creds: SharedCredentialsProvider,
}

impl Realtime {
    /// None when EVENTS_HTTP_DOMAIN isn't set (local development, tests).
    pub fn from_env(config: &aws_config::SdkConfig, http: reqwest::Client) -> Option<Self> {
        let host = std::env::var("EVENTS_HTTP_DOMAIN").ok().filter(|h| !h.is_empty())?;
        Some(Realtime {
            http,
            url: format!("https://{host}/event"),
            region: config.region()?.to_string(),
            creds: config.credentials_provider()?,
        })
    }

    /// Sends `event` to `user`'s inbox; failures are logged, never returned.
    pub async fn notify(&self, user: &str, event: &Value) {
        if let Err(e) = self.publish(&format!("/inbox/{user}"), event).await {
            tracing::warn!(user, error = %e, "real-time publish failed");
        }
    }

    async fn publish(&self, channel: &str, event: &Value) -> Result<(), String> {
        // Events are JSON strings inside the request (AppSync Events' format).
        let body = serde_json::to_vec(&json!({ "channel": channel, "events": [event.to_string()] })).map_err(|e| e.to_string())?;
        let identity: Identity = self.creds.provide_credentials().await.map_err(|e| e.to_string())?.into();
        let params = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.region)
            .name("appsync")
            .time(SystemTime::now())
            .settings(SigningSettings::default())
            .build()
            .map_err(|e| e.to_string())?
            .into();
        let headers = [("content-type", "application/json")];
        let signable = SignableRequest::new("POST", &self.url, headers.into_iter(), SignableBody::Bytes(&body)).map_err(|e| e.to_string())?;
        let (signed, _) = sign(signable, &params).map_err(|e| e.to_string())?.into_parts();

        let mut req = self.http.post(&self.url).header("content-type", "application/json");
        for (name, value) in signed.headers() {
            req = req.header(name, value);
        }
        let res = req.body(body).send().await.map_err(|e| e.to_string())?;
        let status = res.status();
        let text = res.text().await.unwrap_or_default();
        // A 200 can still list failed events.
        if !status.is_success() || serde_json::from_str::<Value>(&text).is_ok_and(|v| v["failed"].as_array().is_some_and(|f| !f.is_empty())) {
            return Err(format!("{status}: {text}"));
        }
        Ok(())
    }
}
