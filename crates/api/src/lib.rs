//! HTTP API for the personal training app: one Rust Lambda behind an API
//! Gateway HTTP API, routing with axum. See docs/ARCHITECTURE.md.
//! `main.rs` wires this router to the Lambda runtime; tests drive it directly.

pub mod auth;
pub mod error;
pub mod mail;
pub mod push;
pub mod realtime;
pub mod routes;
pub mod store;
pub mod sweep;
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
    /// Live delivery to open apps (EVENTS_HTTP_DOMAIN); None turns it off.
    pub realtime: Option<realtime::Realtime>,
    /// Web Push (VAPID_PARAM); None turns it off.
    pub push: Option<push::Push>,
    /// Password changes (USER_POOL_ID, USER_POOL_CLIENT_ID); None turns them off.
    pub cognito: Option<routes::password::Cognito>,
    /// Shared HTTP client (food lookups; push and realtime borrow it too).
    pub http: reqwest::Client,
    /// USDA FoodData Central key (a free data.gov signup): FDC_PARAM, an SSM
    /// SecureString, or FDC_API_KEY locally; None turns /foods/search off. Never ship it in the web app: keys found
    /// in public code get deactivated.
    pub fdc_key: Option<String>,
    /// Cloudflare Turnstile secret (TURNSTILE_PARAM, an SSM SecureString, or
    /// TURNSTILE_SECRET in tests); None leaves the coach-request form without
    /// a human check, and auto-approve off (routes/requests.rs).
    pub turnstile_secret: Option<String>,
}

pub fn router(state: AppState) -> Router {
    use routes::*;
    Router::new()
        .route("/health", get(|| async { Json(json!({ "ok": true })) }))
        // identity + onboarding
        .route("/me", get(me::get_me).put(me::update_me))
        .route("/me/password", post(password::change))
        .route("/me/consent", post(me::consent))
        .route("/me/export", get(account::export))
        .route("/me/account", delete(account::delete))
        .route("/invites", get(invites::list).post(invites::create))
        .route("/invites/{code}", get(invites::peek).delete(invites::revoke))
        .route("/invites/{code}/public", get(invites::public))
        .route("/invites/{code}/accept", post(invites::accept))
        .route("/invites/{code}/resend", post(invites::resend))
        // admin: coach invites by email (no open coach signup)
        .route("/admin/register", post(admin::register))
        .route("/admin/coaches", get(admin::coaches))
        .route("/admin/coaches/{id}", get(coaches::admin_get).delete(account::delete_coach))
        .route("/admin/coaches/{id}/hidden", put(coaches::admin_hide))
        .route("/admin/coaches/{id}/suspended", put(account::suspend_coach))
        .route("/admin/invites", get(admin::list_invites).post(admin::create_invite))
        .route("/admin/invites/{code}", delete(admin::revoke_invite))
        .route("/admin/invites/{code}/resend", post(admin::resend_invite))
        // coach request queue: anonymous ask-to-join from the landing page,
        // the admin's queue/approve/deny/auto-approve behind it
        .route("/public/coach-requests", post(requests::request))
        .route("/admin/coach-requests", get(requests::queue))
        .route("/admin/coach-requests/settings", get(requests::get_settings).put(requests::put_settings))
        .route("/admin/coach-requests/{email}/invite", post(requests::invite_route))
        .route("/admin/coach-requests/{email}/deny", post(requests::deny))
        // client request queues: a coach's own sign-up link (#/request/{coachId},
        // for their website) lands requests in that coach's queue
        .route("/public/join/{coach}", get(requests::coach_page).post(requests::client_request))
        .route("/requests", get(requests::coach_queue))
        .route("/requests/settings", get(requests::coach_get_settings).put(requests::coach_put_settings))
        .route("/requests/{email}/invite", post(requests::coach_invite))
        .route("/requests/{email}/deny", post(requests::coach_deny))
        // coach profiles and the public directory (no sign-in for /public)
        .route("/me/coach-profile", get(coaches::get_mine).put(coaches::put_mine))
        .route("/me/coach-profile/photo", post(coaches::photo_upload))
        .route("/public/coaches", get(coaches::public_list))
        .route("/public/coaches/{id}", get(coaches::public_one))
        // coach library
        .route("/exercises", get(exercises::list).post(exercises::create))
        .route("/exercises/{id}", put(exercises::update).delete(exercises::remove))
        .route("/exercises/{id}/demo-upload", post(exercises::demo_upload))
        .route("/exercises/{id}/demo-confirm", post(exercises::demo_confirm))
        .route("/programs", get(programs::list).post(programs::create))
        // the head coach's team of assistant coaches
        .route("/team", get(team::list))
        .route("/team/invites", post(team::invite))
        .route("/team/invites/{code}", delete(team::revoke_invite))
        .route("/team/{uid}", put(team::update).delete(team::remove))
        .route("/me/team", get(team::mine))
        // service credits: the coach's plans and settings, a client's grants and uses
        .route("/credits/settings", get(credits::get_settings).put(credits::put_settings))
        .route("/plans", get(credits::list_plans).post(credits::create_plan))
        .route("/plans/{id}", put(credits::update_plan).delete(credits::delete_plan))
        .route("/clients/{cid}/credits", get(credits::get_client).post(credits::grant_credits))
        .route("/clients/{cid}/credits/plan", post(credits::grant_plan))
        .route("/clients/{cid}/credits/{gid}", put(credits::edit_grant).delete(credits::revoke))
        // recipes, recipe books and meal plans
        .route("/recipes", get(recipes::list).post(recipes::create))
        .route("/recipes/import", post(recipes::import))
        .route("/recipes/books", get(recipes::list_books).post(recipes::create_book))
        .route("/recipes/books/{bid}", put(recipes::update_book).delete(recipes::delete_book))
        .route("/recipes/{rid}", get(recipes::get_one).put(recipes::update).delete(recipes::remove))
        .route("/clients/{cid}/recipes", get(recipes::for_client).post(recipes::create_own))
        .route("/clients/{cid}/recipes/{rid}", put(recipes::update_own).delete(recipes::remove_own))
        .route("/mealplans", get(recipes::list_plans).post(recipes::create_plan))
        .route("/mealplans/{id}", get(recipes::get_plan).put(recipes::update_plan).delete(recipes::delete_plan))
        .route("/clients/{cid}/mealplan", get(recipes::for_day).put(recipes::assign).delete(recipes::unassign))
        .route("/programs/starters", get(programs::starters))
        .route("/programs/starters/{key}/copy", post(programs::copy_starter))
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
        .route("/clients/{cid}/attachments", post(comments::attachment))
        .route("/clients/{cid}/messages/read", post(messages::mark_read))
        .route("/messages", get(messages::inbox))
        .route("/checkin-template", get(checkins::get_template).put(checkins::put_template))
        .route("/intake-template", get(checkins::get_intake_template).put(checkins::put_intake_template))
        .route("/clients/{cid}/checkins", get(checkins::list))
        .route("/clients/{cid}/checkins/{week}", put(checkins::put))
        .route("/automation", get(automation::get).put(automation::put))
        .route("/availability", get(sessions::get_availability).put(sessions::put_availability))
        .route("/clients/{cid}/slots", get(sessions::slots))
        .route("/sessions", get(sessions::list))
        .route("/clients/{cid}/sessions", post(sessions::book))
        .route("/clients/{cid}/sessions/{sid}/cancel", post(sessions::cancel))
        .route("/me/calendar", get(sessions::my_calendar))
        .route("/me/calendar/reset", post(sessions::reset_calendar))
        .route("/cal/{file}", get(sessions::feed))
        .route("/usage", get(usage::mine).post(usage::record))
        .route("/admin/coaches/{id}/usage", get(usage::for_admin))
        .route("/admin/coaches/{id}/clients", get(account::coach_clients))
        .route("/admin/clients/{cid}/move", post(account::move_client))
        .route("/alerts", get(alerts::list))
        .route("/alerts/settings", put(alerts::put_settings))
        .route("/alerts/{id}/done", post(alerts::done))
        .route("/snippets", get(snippets::list).post(snippets::create))
        .route("/snippets/{id}", put(snippets::update).delete(snippets::remove))
        .route("/suggestions", get(suggestions::list))
        .route("/suggestions/{id}/accept", post(suggestions::accept))
        .route("/suggestions/{id}/dismiss", post(suggestions::dismiss))
        .route("/clients/{cid}/habits", get(habits::list).post(habits::create))
        .route("/clients/{cid}/habits/{hid}", put(habits::update))
        .route("/clients/{cid}/habits/{hid}/days/{date}", put(habits::log_day))
        // food journal: client writes meals, coach searches and estimates
        .route("/clients/{cid}/meals", get(nutrition::list).post(nutrition::create))
        .route("/clients/{cid}/meals/import", post(nutrition::import))
        .route("/clients/{cid}/meals/{mid}", put(nutrition::put_meal).delete(nutrition::remove))
        .route("/clients/{cid}/nutrition/target", get(nutrition::get_target).put(nutrition::put_target))
        .route("/foods/search", get(nutrition::search))
        .route("/foods", get(nutrition::list_foods).post(nutrition::create_food))
        .route("/foods/{fid}", delete(nutrition::remove_food))
        .route("/clients/{cid}/maxes", get(maxes::list))
        .route("/clients/{cid}/records", get(records::list))
        .route("/clients/{cid}/records/{key}", delete(records::remove))
        .route("/clients/{cid}/maxes/{eid}", put(maxes::put).delete(maxes::remove))
        .route("/clients/{cid}/metrics", get(metrics::list).post(metrics::create))
        .route("/clients/{cid}/baseline", get(metrics::get_baseline).put(metrics::put_baseline))
        .route("/clients/{cid}/metrics/{mid}", delete(metrics::remove))
        .route("/clients/{cid}/photos", get(metrics::list_photos).post(metrics::create_photo))
        .route("/clients/{cid}/photos/{pid}", delete(metrics::remove_photo))
        .route("/clients/{cid}/photos/{pid}/confirm", post(metrics::confirm_photo))
        .route("/clients/{cid}/videos", get(videos::list).post(videos::create))
        .route("/clients/{cid}/videos/{vid}/parts", post(videos::parts))
        .route("/clients/{cid}/videos/{vid}/complete", post(videos::complete))
        .route("/clients/{cid}/videos/{vid}", delete(videos::remove))
        .route("/clients/{cid}/videos/{vid}/reviewed", post(videos::reviewed))
        .route("/videos/queue", get(videos::queue))
        // coach replies to set videos (VR-1..VR-3)
        .route("/clients/{cid}/videos/{vid}/replies", post(replies::create))
        .route("/clients/{cid}/replies", get(replies::list))
        .route("/clients/{cid}/replies/{rid}", delete(replies::remove))
        .route("/clients/{cid}/replies/{rid}/parts", post(replies::parts))
        .route("/clients/{cid}/replies/{rid}/complete", post(replies::complete))
        .route("/clients/{cid}/replies/{rid}/seen", post(replies::seen))
        // push notifications (MS-2)
        .route("/push/key", get(notify::public_key))
        .route("/me/push", post(notify::subscribe).delete(notify::unsubscribe))
        // API Gateway owns CORS in AWS; this layer only matters for local dev.
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
        .with_state(state)
}

impl AppState {
    pub async fn from_env() -> Self {
        let config = aws_config::load_from_env().await;
        // Never follows redirects: push endpoints come from browsers (push.rs).
        let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().expect("HTTP client");
        let app_url = std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8080".into());
        AppState {
            store: Store {
                ddb: aws_sdk_dynamodb::Client::new(&config),
                table: std::env::var("TABLE_NAME").expect("TABLE_NAME"),
            },
            s3: aws_sdk_s3::Client::new(&config),
            ses: aws_sdk_sesv2::Client::new(&config),
            invite_from: std::env::var("INVITE_FROM").ok().filter(|f| f.contains('@')),
            realtime: realtime::Realtime::from_env(&config, http.clone()),
            cognito: routes::password::Cognito::from_env(&config),
            push: push::Push::from_env(&config, http.clone(), &app_url).await,
            app_url,
            http: http.clone(),
            fdc_key: util::secret(&config, "FDC_API_KEY", "FDC_PARAM").await,
            turnstile_secret: util::secret(&config, "TURNSTILE_SECRET", "TURNSTILE_PARAM").await,
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
