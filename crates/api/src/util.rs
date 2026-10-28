use std::collections::BTreeMap;

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

/// Photos are shrunk in the browser first, so this is generous.
pub const MAX_PHOTO_BYTES: i64 = 15 * 1024 * 1024;
pub const MAX_VIDEO_BYTES: i64 = 250 * 1024 * 1024;

pub fn check_size(size: i64, max: i64) -> ApiResult<()> {
    if size <= 0 || size > max {
        return Err(bad_request(format!("file must be 1 byte to {} MB", max / (1024 * 1024))));
    }
    Ok(())
}

/// S3 tag on an upload that has not been confirmed yet. The media bucket's
/// lifecycle rule expires objects still carrying it after two days, so
/// abandoned uploads clean themselves up (see template.yaml).
pub const PENDING_TAG: &str = "state=pending";

/// A presigned upload: the browser PUTs to `url` sending exactly `headers`.
pub struct Upload {
    pub url: String,
    pub headers: BTreeMap<String, String>,
}

/// A PUT URL for exactly `size` bytes. Content-Length is one of the signed
/// headers, so S3 refuses an upload of any other size: the size checked here
/// is the size that lands. A `tagging` value is signed too, so the browser
/// must send it (it is in `headers`).
pub async fn presign_put(state: &AppState, key: &str, content_type: &str, size: i64, tagging: Option<&str>) -> ApiResult<Upload> {
    let req = state
        .s3
        .put_object()
        .bucket(&state.bucket)
        .key(key)
        .content_type(content_type)
        .content_length(size)
        .set_tagging(tagging.map(str::to_owned))
        .presigned(PresigningConfig::expires_in(state.presign_ttl)?)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("presign put: {e}")))?;
    Ok(browser_upload(&req))
}

/// A PUT URL for one part of a multipart upload, signed for exactly `size`
/// bytes like `presign_put`. S3's response carries the part's ETag, which
/// the browser reads (the media bucket's CORS exposes it) and sends back
/// when completing the upload.
pub async fn presign_part(state: &AppState, key: &str, upload_id: &str, part_number: i32, size: i64) -> ApiResult<Upload> {
    let req = state
        .s3
        .upload_part()
        .bucket(&state.bucket)
        .key(key)
        .upload_id(upload_id)
        .part_number(part_number)
        .content_length(size)
        .presigned(PresigningConfig::expires_in(state.presign_ttl)?)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("presign part: {e}")))?;
    Ok(browser_upload(&req))
}

fn browser_upload(req: &aws_sdk_s3::presigning::PresignedRequest) -> Upload {
    // Browsers set Content-Length themselves and refuse to let scripts do it.
    let headers = req
        .headers()
        .filter(|(k, _)| !k.eq_ignore_ascii_case("content-length"))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    Upload { url: req.uri().to_string(), headers }
}

/// Removes the pending tag from a confirmed upload, so the lifecycle rule
/// that expires unconfirmed uploads leaves it alone.
pub async fn untag(state: &AppState, key: &str) -> ApiResult<()> {
    state
        .s3
        .delete_object_tagging()
        .bucket(&state.bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("s3 untag: {e}")))?;
    Ok(())
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

/// Whether an object exists in the media bucket.
pub async fn s3_object_exists(state: &AppState, key: &str) -> ApiResult<bool> {
    Ok(s3_object_size(state, key).await?.is_some())
}

/// The size of an object in the media bucket, or None if it doesn't exist.
/// HeadObject reports a missing object as the NotFound error variant; other
/// errors are surfaced.
pub async fn s3_object_size(state: &AppState, key: &str) -> ApiResult<Option<i64>> {
    match state.s3.head_object().bucket(&state.bucket).key(key).send().await {
        Ok(out) => Ok(Some(out.content_length().unwrap_or(0))),
        Err(e) if e.as_service_error().is_some_and(|se| se.is_not_found()) => Ok(None),
        Err(e) => Err(crate::error::ApiError::Internal(format!("s3 head: {e}"))),
    }
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
    if scope.by_coach {
        audit_client(state, ctx, &scope.pk(), action).await;
    }
}

/// `audit` for a coach reading data of the client whose partition is
/// `client_pk`, where there is no single `ClientScope` (e.g. the video
/// review queue spans all of a coach's clients).
pub async fn audit_client(state: &AppState, ctx: &Ctx, client_pk: &str, action: &str) {
    let now = Utc::now();
    let entry = AuditEntry {
        actor_id: ctx.user_id(),
        actor_name: &ctx.profile.name,
        action,
        at: now,
        ttl: now.timestamp() + 365 * 24 * 3600,
    };
    let sk = format!("AUDIT#{}#{}", now.to_rfc3339(), &new_id()[..8]);
    if let Err(e) = state.store.put(client_pk, &sk, &entry).await {
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

    /// The upload size limit depends on the SDK signing Content-Length.
    #[tokio::test]
    async fn presigned_put_signs_the_content_length() {
        use aws_sdk_s3::config::{BehaviorVersion, Credentials, Region};
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .credentials_provider(Credentials::new("AKID", "SECRET", None, None, "test"))
            .build();
        let ddb = aws_sdk_dynamodb::Client::from_conf(
            aws_sdk_dynamodb::Config::builder().behavior_version(BehaviorVersion::latest()).region(Region::new("us-east-1")).build(),
        );
        let state = crate::AppState {
            store: crate::store::Store { ddb, table: "t".into() },
            s3: aws_sdk_s3::Client::from_conf(conf),
            ses: aws_sdk_sesv2::Client::from_conf(
                aws_sdk_sesv2::Config::builder().behavior_version(BehaviorVersion::latest()).region(Region::new("us-east-1")).build(),
            ),
            invite_from: None,
            app_url: "http://localhost:8080".into(),
            bucket: "b".into(),
            presign_ttl: std::time::Duration::from_secs(60),
            admin_emails: vec![],
            video_retention_days: 180,
            realtime: None,
            push: None,
        };
        let up = super::presign_put(&state, "k", "image/jpeg", 1234, Some(super::PENDING_TAG)).await.unwrap();
        assert!(up.url.contains("X-Amz-SignedHeaders=content-length%3B"), "{}", up.url);
        assert_eq!(up.headers.get("x-amz-tagging").map(String::as_str), Some(super::PENDING_TAG));
        assert!(!up.headers.contains_key("content-length"));
        assert!(super::check_size(0, super::MAX_PHOTO_BYTES).is_err());
        assert!(super::check_size(super::MAX_PHOTO_BYTES + 1, super::MAX_PHOTO_BYTES).is_err());
    }
}


