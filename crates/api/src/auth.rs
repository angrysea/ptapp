//! Identity and authorization.
//!
//! API Gateway's JWT authorizer has already verified the Cognito ID token, so
//! here we only read its claims. Roles and tenancy live in our own Profile
//! item, not in Cognito groups, so a coach/client relationship can change
//! without reissuing tokens.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use lambda_http::request::RequestContext;
use ptcore::model::{Profile, Role};

use crate::error::{ApiError, ApiResult};
use crate::AppState;

/// The authenticated Cognito user (may not have a Profile yet).
#[derive(Debug, Clone)]
pub struct Caller {
    pub sub: String,
    pub email: String,
    /// Cognito's `email_verified` claim; invites are bound to a verified address.
    pub email_verified: bool,
    pub source_ip: Option<String>,
    pub user_agent: Option<String>,
}

impl<S: Send + Sync> FromRequestParts<S> for Caller {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let user_agent = parts
            .headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);

        if let Some(RequestContext::ApiGatewayV2(ctx)) = parts.extensions.get::<RequestContext>() {
            let claims = ctx.authorizer.as_ref().and_then(|a| a.jwt.as_ref()).map(|j| &j.claims);
            if let Some(sub) = claims.and_then(|c| c.get("sub")) {
                let email = claims.and_then(|c| c.get("email")).cloned().unwrap_or_default();
                let email_verified = claims.and_then(|c| c.get("email_verified")).is_some_and(|v| v == "true");
                return Ok(Caller { sub: sub.clone(), email, email_verified, source_ip: ctx.http.source_ip.clone(), user_agent });
            }
        }

        // Local development only (cargo lambda watch): trust an x-dev-user header.
        if std::env::var("DEV_AUTH").as_deref() == Ok("1")
            && let Some(user) = parts.headers.get("x-dev-user").and_then(|v| v.to_str().ok())
        {
            return Ok(Caller { sub: user.to_string(), email: format!("{user}@dev.local"), email_verified: true, source_ip: None, user_agent });
        }
        Err(ApiError::Unauthorized)
    }
}

/// A caller who has completed onboarding (has a Profile).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub caller: Caller,
    pub profile: Profile,
}

impl FromRequestParts<AppState> for Ctx {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let caller = Caller::from_request_parts(parts, state).await?;
        let profile: Profile = state
            .store
            .get(&format!("USER#{}", caller.sub), "PROFILE")
            .await?
            .ok_or(ApiError::Forbidden("onboarding required"))?;
        if profile.suspended_at.is_some() {
            return Err(ApiError::Forbidden("this account is suspended; contact PT Coach support"));
        }
        Ok(Ctx { caller, profile })
    }
}

impl Ctx {
    pub fn user_id(&self) -> &str {
        &self.profile.user_id
    }

    pub fn is_coach(&self) -> bool {
        self.profile.role == Role::Coach
    }

    pub fn require_admin(&self) -> ApiResult<&str> {
        if self.profile.role == Role::Admin {
            Ok(&self.profile.user_id)
        } else {
            Err(ApiError::Forbidden("admin only"))
        }
    }

    pub fn require_coach(&self) -> ApiResult<&str> {
        if self.is_coach() {
            Ok(&self.profile.user_id)
        } else {
            Err(ApiError::Forbidden("coach only"))
        }
    }
}

/// Resolved access to one client's data.
pub struct ClientScope {
    pub client: Profile,
    pub by_coach: bool,
}

impl ClientScope {
    pub fn pk(&self) -> String {
        format!("CLIENT#{}", self.client.user_id)
    }

    /// The client's local date, which is what their workouts are dated in.
    pub fn today(&self) -> chrono::NaiveDate {
        self.client.today(chrono::Utc::now())
    }

    pub fn actor_role(&self) -> Role {
        if self.by_coach { Role::Coach } else { Role::Client }
    }
}

/// A client can see only their own data; a coach only their own clients' data.
pub async fn client_scope(state: &AppState, ctx: &Ctx, client_id: &str) -> ApiResult<ClientScope> {
    if ctx.profile.role == Role::Client {
        if ctx.user_id() == client_id {
            return Ok(ClientScope { client: ctx.profile.clone(), by_coach: false });
        }
        return Err(ApiError::Forbidden("not your data"));
    }
    if ctx.is_coach() {
        let client: Profile = state
            .store
            .get(&format!("USER#{client_id}"), "PROFILE")
            .await?
            .ok_or(ApiError::NotFound("client"))?;
        if client.role == Role::Client && client.coach_id == ctx.user_id() {
            return Ok(ClientScope { client, by_coach: true });
        }
    }
    Err(ApiError::Forbidden("not your client"))
}
