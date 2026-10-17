#[tokio::main]
async fn main() -> Result<(), lambda_http::Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();
    let state = api::AppState::from_env().await;
    lambda_http::run(api::router(state)).await
}
