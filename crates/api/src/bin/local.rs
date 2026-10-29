//! Local development server: the same router as the Lambda, served over plain
//! HTTP on :9000 with DEV_AUTH enabled. Point AWS_ENDPOINT_URL at an emulator
//! (moto via `docker compose up`, DynamoDB Local + an S3 mock) and it creates the table and
//! bucket on startup. See docs/DEVELOPMENT.md.

use aws_sdk_dynamodb::types::{AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType};
use aws_sdk_s3::types::{CorsConfiguration, CorsRule};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::from_default_env()).init();
    // SAFETY: single-threaded at this point; nothing else reads the environment yet.
    unsafe {
        std::env::set_var("DEV_AUTH", "1");
        if std::env::var("TABLE_NAME").is_err() {
            std::env::set_var("TABLE_NAME", "ptapp-local");
        }
        if std::env::var("MEDIA_BUCKET").is_err() {
            std::env::set_var("MEDIA_BUCKET", "ptapp-local-media");
        }
        // Invitation links point at the local web server; INVITE_FROM is
        // unset, so nothing is emailed.
        if std::env::var("APP_URL").is_err() {
            std::env::set_var("APP_URL", "http://localhost:8080");
        }
        // Sign in as dev user "admin" to set up the admin account.
        if std::env::var("ADMIN_EMAILS").is_err() {
            std::env::set_var("ADMIN_EMAILS", "admin@dev.local");
        }
    }
    let mut state = api::AppState::from_env().await;

    if std::env::var("AWS_ENDPOINT_URL").is_ok() {
        let config = aws_config::load_from_env().await;
        state.s3 = aws_sdk_s3::Client::from_conf(aws_sdk_s3::config::Builder::from(&config).force_path_style(true).build());
        let created = state
            .store
            .ddb
            .create_table()
            .table_name(&state.store.table)
            .billing_mode(BillingMode::PayPerRequest)
            .attribute_definitions(AttributeDefinition::builder().attribute_name("pk").attribute_type(ScalarAttributeType::S).build().unwrap())
            .attribute_definitions(AttributeDefinition::builder().attribute_name("sk").attribute_type(ScalarAttributeType::S).build().unwrap())
            .key_schema(KeySchemaElement::builder().attribute_name("pk").key_type(KeyType::Hash).build().unwrap())
            .key_schema(KeySchemaElement::builder().attribute_name("sk").key_type(KeyType::Range).build().unwrap())
            .send()
            .await;
        if created.is_ok() {
            println!("created table {}", state.store.table);
        }
        let _ = state.s3.create_bucket().bucket(&state.bucket).send().await;
        // Same CORS as the media bucket in template.yaml, so browser uploads
        // (which read each part's ETag) work against the emulator too.
        let rule = CorsRule::builder()
            .allowed_methods("GET")
            .allowed_methods("PUT")
            .allowed_origins("*")
            .allowed_headers("*")
            .expose_headers("ETag")
            .build()
            .unwrap();
        let _ = state
            .s3
            .put_bucket_cors()
            .bucket(&state.bucket)
            .cors_configuration(CorsConfiguration::builder().cors_rules(rule).build().unwrap())
            .send()
            .await;
    }

    // PORT runs a second copy beside one already on :9000.
    let port = std::env::var("PORT").unwrap_or_else(|_| "9000".into());
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}")).await.unwrap_or_else(|e| panic!("bind :{port}: {e}"));
    println!("Local API listening on http://localhost:{port} (DEV_AUTH on: send x-dev-user)");
    axum::serve(listener, api::router(state)).await.unwrap();
}
