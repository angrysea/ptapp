//! HTTP API for the personal training app: one Rust Lambda behind an API
//! Gateway HTTP API, routing with axum. See docs/ARCHITECTURE.md.
//! `main.rs` wires this router to the Lambda runtime; tests drive it directly.

pub mod auth;
pub mod error;
pub mod mail;
pub mod routes;
pub mod store;
pub mod util;

use std::time::Duration;

use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde_json::json;
use tower_http::cors::{Any, CorsLayer};

use crate::store::Store;

#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub s3: aws_sdk_s3::Client,
    pub ses: aws_sdk_sesv2::Client,
    /// Sender for invitation emails (INVITE_FROM); None turns emailing off.
    pub invite_from: Option<String>,
    /// The web app's base URL (APP_URL), for links in emails.
    pub app_url: String,
    pub bucket: String,
    pub presign_ttl: Duration,
    /// Lowercased emails allowed to set up an admin account (ADMIN_EMAILS,
    /// comma-separated). Only needed to bootstrap: an admin profile stays
    /// once created.
    pub admin_emails: Vec<String>,
    /// Days after upload that reviewed set videos are deleted (template
    /// parameter VideoRetentionDays, which also sets the S3 lifecycle rule).
    pub video_retention_days: i64,
}

pub fn router(state: AppState) -> Router {
    use routes::*;
    Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        // identity + onboarding
        .route("/me", get(me::get_me).put(me::update_me))
        .route("/invites", get(invites::list).post(invites::create))
        .route("/invites/{code}", get(invites::peek).delete(invites::revoke))
        .route("/invites/{code}/public", get(invites::public))
        .route("/invites/{code}/accept", post(invites::accept))
        // admin: coach invites by email (no open coach signup)
        .route("/admin/register", post(admin::register))
        .route("/admin/coaches", get(admin::coaches))
        .route("/admin/invites", get(admin::list_invites).post(admin::create_invite))
        .route("/admin/invites/{code}", delete(admin::revoke_invite))
        .route("/admin/invites/{code}/resend", post(admin::resend_invite))
        // coach library
        .route("/exercises", get(exercises::list).post(exercises::create))
        .route("/exercises/{id}", put(exercises::update).delete(exercises::remove))
        .route("/exercises/{id}/demo-upload", post(exercises::demo_upload))
        .route("/exercises/{id}/demo-confirm", post(exercises::demo_confirm))
        .route("/programs", get(programs::list).post(programs::create))
        .route("/programs/{id}", get(programs::get_one).put(programs::update).delete(programs::remove))
        .route("/programs/{id}/copy", post(programs::copy))
        // clients
        .route("/clients", get(clients::roster))
        .route("/clients/{cid}", get(clients::detail))
        .route("/clients/{cid}/intake", get(clients::get_intake).put(clients::put_intake))
        .route("/clients/{cid}/waiver", get(clients::get_waiver).post(clients::sign_waiver))
        .route("/clients/{cid}/dashboard", get(clients::dashboard))
        .route("/clients/{cid}/assignments", get(training::list_assignments).post(training::assign))
        .route("/clients/{cid}/assignments/{aid}", delete(training::unassign))
        .route("/clients/{cid}/schedule", get(training::schedule))
        .route("/clients/{cid}/logs", get(training::logs))
        .route("/clients/{cid}/logs/{lid}", put(training::put_log))
        .route("/clients/{cid}/comments", get(comments::list).post(comments::create))
        .route("/clients/{cid}/metrics", get(metrics::list).post(metrics::create))
        .route("/clients/{cid}/metrics/{mid}", delete(metrics::remove))
        .route("/clients/{cid}/photos", get(metrics::list_photos).post(metrics::create_photo))
        .route("/clients/{cid}/photos/{pid}", delete(metrics::remove_photo))
        .route("/clients/{cid}/videos", get(videos::list).post(videos::create))
        .route("/clients/{cid}/videos/{vid}/parts", post(videos::parts))
        .route("/clients/{cid}/videos/{vid}/complete", post(videos::complete))
        .route("/clients/{cid}/videos/{vid}", delete(videos::remove))
        .route("/clients/{cid}/videos/{vid}/reviewed", post(videos::reviewed))
        .route("/videos/queue", get(videos::queue))
        // API Gateway owns CORS in AWS; this layer only matters for local dev.
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state)
}

impl AppState {
    pub async fn from_env() -> Self {
        let config = aws_config::load_from_env().await;
        AppState {
            store: Store {
                ddb: aws_sdk_dynamodb::Client::new(&config),
                table: std::env::var("TABLE_NAME").expect("TABLE_NAME"),
            },
            s3: aws_sdk_s3::Client::new(&config),
            ses: aws_sdk_sesv2::Client::new(&config),
            invite_from: std::env::var("INVITE_FROM").ok().filter(|f| f.contains('@')),
            app_url: std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8080".into()),
            bucket: std::env::var("MEDIA_BUCKET").expect("MEDIA_BUCKET"),
            presign_ttl: Duration::from_secs(900),
            video_retention_days: std::env::var("VIDEO_RETENTION_DAYS").ok().and_then(|d| d.parse().ok()).unwrap_or(180),
            admin_emails: std::env::var("ADMIN_EMAILS")
                .unwrap_or_default()
                .split(',')
                .map(|e| e.trim().to_lowercase())
                .filter(|e| e.contains('@'))
                .collect(),
        }
    }

    /// Whether this signed-in user may set up an admin account.
    pub fn is_admin_email(&self, caller: &auth::Caller) -> bool {
        caller.email_verified && self.admin_emails.contains(&caller.email.trim().to_lowercase())
    }
}
