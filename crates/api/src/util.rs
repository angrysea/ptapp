use aws_sdk_s3::presigning::PresigningConfig;
use chrono::{NaiveDate, Utc};
use serde::Serialize;

use crate::auth::{ClientScope, Ctx};
use crate::error::{bad_request, ApiResult};
use crate::AppState;

pub fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Short id that sorts by date: `2026-10-17-1a2b3c4d`.
pub fn dated_id(date: NaiveDate) -> String {
    format!("{date}-{}", &new_id()[..8])
}

/// 10-character invite code without look-alike characters.
pub fn invite_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    uuid::Uuid::new_v4().as_bytes()[..10]
        .iter()
        .map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char)
        .collect()
}

pub fn today() -> NaiveDate {
    Utc::now().date_naive()
}

pub fn parse_date(s: &str) -> ApiResult<NaiveDate> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad_request(format!("bad date '{s}', expected YYYY-MM-DD")))
}

pub fn ext_for(content_type: &str, allowed: &[(&'static str, &'static str)]) -> ApiResult<&'static str> {
    allowed
        .iter()
        .find(|(ct, _)| *ct == content_type)
        .map(|(_, ext)| *ext)
        .ok_or_else(|| bad_request(format!("unsupported content type {content_type}")))
}

pub const IMAGE_TYPES: &[(&str, &str)] = &[("image/jpeg", "jpg"), ("image/png", "png"), ("image/webp", "webp")];
pub const VIDEO_TYPES: &[(&str, &str)] = &[("video/mp4", "mp4"), ("video/quicktime", "mov"), ("video/webm", "webm")];

pub async fn presign_put(state: &AppState, key: &str, content_type: &str) -> ApiResult<String> {
    let req = state
        .s3
        .put_object()
        .bucket(&state.bucket)
        .key(key)
        .content_type(content_type)
        .presigned(PresigningConfig::expires_in(state.presign_ttl)?)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("presign put: {e}")))?;
    Ok(req.uri().to_string())
}

pub async fn presign_get(state: &AppState, key: &str) -> ApiResult<String> {
    let req = state
        .s3
        .get_object()
        .bucket(&state.bucket)
        .key(key)
        .presigned(PresigningConfig::expires_in(state.presign_ttl)?)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("presign get: {e}")))?;
    Ok(req.uri().to_string())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AuditEntry<'a> {
    actor_id: &'a str,
    actor_name: &'a str,
    action: &'a str,
    at: chrono::DateTime<Utc>,
    ttl: i64,
}

/// Records a coach reading a client's sensitive data (security NFR: audit
/// log of coach access). Kept for a year; failures never block the request.
pub async fn audit(state: &AppState, ctx: &Ctx, scope: &ClientScope, action: &str) {
    if !scope.by_coach {
        return;
    }
    let now = Utc::now();
    let entry = AuditEntry {
        actor_id: ctx.user_id(),
        actor_name: &ctx.profile.name,
        action,
        at: now,
        ttl: now.timestamp() + 365 * 24 * 3600,
    };
    let sk = format!("AUDIT#{}#{}", now.to_rfc3339(), &new_id()[..8]);
    if let Err(e) = state.store.put(&scope.pk(), &sk, &entry).await {
        tracing::warn!(?e, "audit write failed");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn invite_codes_are_10_unambiguous_chars() {
        let c = super::invite_code();
        assert_eq!(c.len(), 10);
        assert!(!c.contains(['0', 'O', '1', 'I', 'L']));
    }
}
