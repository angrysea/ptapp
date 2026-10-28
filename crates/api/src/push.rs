//! Web Push (MS-2): notifications through the browser makers' push services
//! (Apple, Google, Mozilla), signed with this app's VAPID key. The private
//! key is an SSM SecureString created once by scripts/deploy.sh; its name is
//! VAPID_PARAM. Pure Rust (web-push-native, rustls), so the arm64 Lambda
//! needs no OpenSSL.

use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::Value;
use web_push_native::jwt_simple::algorithms::{ECDSAP256PublicKeyLike as _, ES256KeyPair};
use web_push_native::WebPushBuilder;

#[derive(Clone)]
pub struct Push {
    http: reqwest::Client,
    key: Arc<ES256KeyPair>,
    /// The VAPID public key, base64url, for `pushManager.subscribe`.
    pub public_key: String,
    /// VAPID contact (`sub` claim): the app's URL.
    contact: String,
}

/// What the push service said about a subscription.
#[derive(Debug, PartialEq, Eq)]
pub enum Sent {
    Delivered,
    /// 404/410: the subscription is gone and should be deleted.
    Gone,
}

impl Push {
    /// None when VAPID_PARAM isn't set (local development, tests) or the key
    /// can't be read; the reason is logged.
    pub async fn from_env(config: &aws_config::SdkConfig, http: reqwest::Client, contact: &str) -> Option<Self> {
        let name = std::env::var("VAPID_PARAM").ok().filter(|n| !n.is_empty())?;
        let res = aws_sdk_ssm::Client::new(config).get_parameter().name(&name).with_decryption(true).send().await;
        let key = match res {
            Ok(out) => out.parameter.and_then(|p| p.value)?,
            Err(e) => {
                tracing::warn!(param = %name, error = %aws_sdk_ssm::error::DisplayErrorContext(e), "push disabled: can't read the VAPID key");
                return None;
            }
        };
        Push::from_private_key(&key, http, contact).map_err(|e| tracing::warn!(error = %e, "push disabled")).ok()
    }

    /// `private_key` is the raw 32-byte P-256 scalar, base64url.
    pub fn from_private_key(private_key: &str, http: reqwest::Client, contact: &str) -> Result<Self, String> {
        let bytes = B64.decode(private_key.trim()).map_err(|e| format!("VAPID key: {e}"))?;
        let key = ES256KeyPair::from_bytes(&bytes).map_err(|e| format!("VAPID key: {e}"))?;
        let public_key = B64.encode(key.public_key().public_key().to_bytes_uncompressed());
        Ok(Push { http, key: Arc::new(key), public_key, contact: contact.to_string() })
    }

    /// Sends `payload` (JSON) to one browser subscription as returned by
    /// `PushSubscription.toJSON()`: `{endpoint, keys: {p256dh, auth}}`.
    /// Only HTTPS endpoints of the browser push services are contacted (see
    /// `is_push_service`), since the endpoint comes from the browser and so
    /// from any signed-in user. The push service's reply is logged, not returned.
    pub async fn send(&self, subscription: &Value, payload: &Value) -> Result<Sent, String> {
        let endpoint = subscription["endpoint"].as_str().unwrap_or_default();
        if !is_push_service(endpoint) {
            return Err("not a browser push service endpoint".into());
        }
        let builder: WebPushBuilder = serde_json::from_value(subscription.clone()).map_err(|e| format!("bad subscription: {e}"))?;
        let req = builder
            .with_valid_duration(std::time::Duration::from_secs(24 * 3600))
            .with_vapid(&self.key, &self.contact)
            .build(payload.to_string())
            .map_err(|e| e.to_string())?;
        let req = reqwest::Request::try_from(req).map_err(|e| e.to_string())?;
        let res = self.http.execute(req).await.map_err(|e| e.to_string())?;
        match res.status().as_u16() {
            200..=299 => Ok(Sent::Delivered),
            404 | 410 => Ok(Sent::Gone),
            s => {
                let body = res.text().await.unwrap_or_default();
                tracing::warn!(status = s, %body, "push service refused a notification");
                Err(format!("push service answered {s}"))
            }
        }
    }
}

/// The push services browsers subscribe with: Chrome and Edge on Android
/// (FCM), Firefox (Mozilla), Safari (Apple) and Edge on Windows (WNS).
const PUSH_SERVICES: &[&str] = &["fcm.googleapis.com", "push.services.mozilla.com", "push.apple.com", "notify.windows.com"];

/// Whether `endpoint` is an HTTPS URL on one of `PUSH_SERVICES` or a subdomain of one.
pub fn is_push_service(endpoint: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(endpoint) else { return false };
    let Some(host) = url.host_str() else { return false };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    url.scheme() == "https"
        && url.port().is_none()
        && PUSH_SERVICES.iter().any(|s| host == *s || host.ends_with(&format!(".{s}")))
}

/// Makes a new VAPID key pair: (private, public), both base64url.
pub fn generate_keys() -> (String, String) {
    let key = ES256KeyPair::generate();
    (B64.encode(key.to_bytes()), B64.encode(key.public_key().public_key().to_bytes_uncompressed()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_key_loads_and_gives_the_same_public_key() {
        let (private, public) = generate_keys();
        let push = Push::from_private_key(&private, reqwest::Client::new(), "https://example.com").unwrap();
        assert_eq!(push.public_key, public);
        // An uncompressed P-256 point: 65 bytes, starting 0x04.
        let raw = B64.decode(&public).unwrap();
        assert_eq!((raw.len(), raw[0]), (65, 4));
        assert!(Push::from_private_key("not a key", reqwest::Client::new(), "x").is_err());
    }

    #[test]
    fn only_browser_push_services_are_contacted() {
        for ok in [
            "https://fcm.googleapis.com/fcm/send/abc",
            "https://web.push.apple.com/QGx",
            "https://updates.push.services.mozilla.com/wpush/v2/x",
            "https://wns2-par02p.notify.windows.com/w/?token=x",
            "https://WEB.PUSH.APPLE.COM./x",
        ] {
            assert!(is_push_service(ok), "{ok}");
        }
        for bad in [
            "http://fcm.googleapis.com/x",
            "https://fcm.googleapis.com:8443/x",
            "https://evilpush.apple.com/x",
            "https://push.apple.com.evil.com/x",
            "https://169.254.169.254/latest/meta-data",
            "https://localhost/x",
            "not a url",
            "",
        ] {
            assert!(!is_push_service(bad), "{bad}");
        }
    }

    #[test]
    fn a_browser_subscription_builds_an_encrypted_signed_request() {
        let (private, _) = generate_keys();
        let push = Push::from_private_key(&private, reqwest::Client::new(), "https://example.com").unwrap();
        // A real-shaped subscription (keys from the web-push-native docs).
        let sub = serde_json::json!({
            "endpoint": "https://push.example.com/abc",
            "keys": {
                "p256dh": "BLn9b-VR0ca83knDNZ32dCHGyjJp-1riX9ZTN40MqV8K_LpQmLqxC_DoHvqvFXO_nGdAB4W9dogZb_sM-uV4JbY",
                "auth": "_ordMnz7uTCmrpBTeUV4Bw"
            }
        });
        let builder: WebPushBuilder = serde_json::from_value(sub).unwrap();
        let req = builder.with_vapid(&push.key, &push.contact).build("hi").unwrap();
        assert_eq!(req.headers()["content-encoding"], "aes128gcm");
        assert!(req.headers()["authorization"].to_str().unwrap().starts_with("vapid t="));
    }
}
