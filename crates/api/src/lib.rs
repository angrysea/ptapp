//! HTTP API for the personal training app: one Rust Lambda behind an API
//! Gateway HTTP API, routing with axum. See docs/ARCHITECTURE.md.
//! `main.rs` wires this router to the Lambda runtime; tests drive it directly.

pub mod auth;
pub mod error;
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
    pub bucket: String,
    pub presign_ttl: Duration,
}

pub fn router(state: AppState) -> Router {
    use routes::*;
    Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        // identity + onboarding
        .route("/me", get(me::get_me).put(me::update_me))
        .route("/coach/register", post(me::register_coach))
        .route("/invites", get(invites::list).post(invites::create))
        .route("/invites/{code}", get(invites::peek).delete(invites::revoke))
        .route("/invites/{code}/accept", post(invites::accept))
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
            bucket: std::env::var("MEDIA_BUCKET").expect("MEDIA_BUCKET"),
            presign_ttl: Duration::from_secs(900),
        }
    }
}
