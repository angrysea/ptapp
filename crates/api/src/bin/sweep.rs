//! The hourly sweep's Lambda (api::sweep), run by EventBridge Scheduler.
//! The event's contents don't matter; it only marks the time to run.

use lambda_http::lambda_runtime::{run, service_fn, Error, LambdaEvent};
use serde_json::Value;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();
    let state = api::AppState::from_env().await;
    let state = &state;
    run(service_fn(move |_: LambdaEvent<Value>| async move {
        let report = api::sweep::run(state, chrono::Utc::now()).await;
        tracing::info!(?report, "sweep done");
        Ok::<_, Error>(serde_json::to_value(&report)?)
    }))
    .await
}
