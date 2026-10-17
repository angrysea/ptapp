use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

#[derive(Debug)]
pub enum ApiError {
    BadRequest(String),
    Unauthorized,
    Forbidden(&'static str),
    NotFound(&'static str),
    /// Carries the current server copy so offline clients can reconcile.
    Conflict(Value),
    Internal(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

pub fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError::BadRequest(msg.into())
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::BadRequest(m) => (StatusCode::BAD_REQUEST, json!({ "error": m })),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, json!({ "error": "unauthorized" })),
            ApiError::Forbidden(m) => (StatusCode::FORBIDDEN, json!({ "error": m })),
            ApiError::NotFound(m) => (StatusCode::NOT_FOUND, json!({ "error": format!("{m} not found") })),
            ApiError::Conflict(current) => (StatusCode::CONFLICT, json!({ "error": "conflict", "current": current })),
            ApiError::Internal(m) => {
                tracing::error!(error = %m, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, json!({ "error": "internal error" }))
            }
        };
        (status, Json(body)).into_response()
    }
}

impl<E, R> From<aws_sdk_dynamodb::error::SdkError<E, R>> for ApiError
where
    E: std::error::Error + Send + Sync + 'static,
    R: std::fmt::Debug,
{
    fn from(e: aws_sdk_dynamodb::error::SdkError<E, R>) -> Self {
        ApiError::Internal(format!("{:?}", aws_sdk_dynamodb::error::DisplayErrorContext(e)))
    }
}

impl From<serde_dynamo::Error> for ApiError {
    fn from(e: serde_dynamo::Error) -> Self {
        ApiError::Internal(format!("serde_dynamo: {e}"))
    }
}

impl From<aws_sdk_s3::presigning::PresigningConfigError> for ApiError {
    fn from(e: aws_sdk_s3::presigning::PresigningConfigError) -> Self {
        ApiError::Internal(format!("presign: {e}"))
    }
}
