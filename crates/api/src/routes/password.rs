//! `POST /me/password`: a signed-in user changes their password. The API
//! checks the current password with Cognito (AdminInitiateAuth) before
//! setting the new one (AdminSetUserPassword), so the browser's tokens never
//! need the `aws.cognito.signin.user.admin` scope. That scope would also let a
//! stolen token change the account's email, then reset its password.

use aws_sdk_cognitoidentityprovider::error::ProvideErrorMetadata;
use aws_sdk_cognitoidentityprovider::types::AuthFlowType;
use axum::extract::State;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::Caller;
use crate::error::{bad_request, ApiError, ApiResult};
use crate::AppState;

/// The user pool the API administers passwords in (USER_POOL_ID, USER_POOL_CLIENT_ID).
#[derive(Clone)]
pub struct Cognito {
    pub client: aws_sdk_cognitoidentityprovider::Client,
    pub pool_id: String,
    pub client_id: String,
}

impl Cognito {
    /// None when the pool isn't configured (local development, tests).
    pub fn from_env(config: &aws_config::SdkConfig) -> Option<Self> {
        let pool_id = std::env::var("USER_POOL_ID").ok().filter(|v| !v.is_empty())?;
        let client_id = std::env::var("USER_POOL_CLIENT_ID").ok().filter(|v| !v.is_empty())?;
        Some(Cognito { client: aws_sdk_cognitoidentityprovider::Client::new(config), pool_id, client_id })
    }
}

#[derive(Deserialize)]
pub struct ChangePassword {
    current: String,
    new: String,
}

pub async fn change(State(st): State<AppState>, caller: Caller, Json(body): Json<ChangePassword>) -> ApiResult<Json<Value>> {
    let Some(cog) = &st.cognito else { return Err(bad_request("changing passwords isn't available here")) };
    if body.current.is_empty() || body.new.is_empty() {
        return Err(bad_request("enter your current and new passwords"));
    }
    if body.new == body.current {
        return Err(bad_request("choose a password different from your current one"));
    }
    // The pool's username is the user's sub (email is a sign-in alias).
    let auth = cog
        .client
        .admin_initiate_auth()
        .user_pool_id(&cog.pool_id)
        .client_id(&cog.client_id)
        .auth_flow(AuthFlowType::AdminUserPasswordAuth)
        .auth_parameters("USERNAME", &caller.sub)
        .auth_parameters("PASSWORD", &body.current)
        .send()
        .await;
    // A challenge (e.g. the user's MFA code) still means the password was right.
    if let Err(e) = auth {
        return Err(match e.code() {
            Some("NotAuthorizedException") | Some("UserNotFoundException") => bad_request("your current password isn’t right"),
            Some("TooManyRequestsException") | Some("LimitExceededException") => bad_request("too many attempts; wait a few minutes and try again"),
            _ => ApiError::Internal(format!("cognito auth: {}", aws_sdk_cognitoidentityprovider::error::DisplayErrorContext(e))),
        });
    }
    let set = cog
        .client
        .admin_set_user_password()
        .user_pool_id(&cog.pool_id)
        .username(&caller.sub)
        .password(&body.new)
        .permanent(true)
        .send()
        .await;
    if let Err(e) = set {
        return Err(match e.code() {
            // The pool's policy (template.yaml) explains what's wrong.
            Some("InvalidPasswordException") => bad_request(e.message().unwrap_or("that password isn’t allowed").to_string()),
            _ => ApiError::Internal(format!("cognito set password: {}", aws_sdk_cognitoidentityprovider::error::DisplayErrorContext(e))),
        });
    }
    tracing::info!(user = %caller.sub, "password changed");
    Ok(Json(json!({ "ok": true })))
}
