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
use ptcore::team::{Member, Perm};

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

/// The caller's IP address, for routes that take no sign-in: API Gateway's,
/// or locally (DEV_AUTH) an `x-dev-ip` header so tests can stand in for
/// different senders. None when neither is there.
#[derive(Debug, Clone)]
pub struct SourceIp(pub Option<String>);

impl<S: Send + Sync> FromRequestParts<S> for SourceIp {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        if let Some(RequestContext::ApiGatewayV2(ctx)) = parts.extensions.get::<RequestContext>() {
            return Ok(SourceIp(ctx.http.source_ip.clone()));
        }
        let dev = std::env::var("DEV_AUTH").as_deref() == Ok("1");
        Ok(SourceIp(dev.then(|| parts.headers.get("x-dev-ip").and_then(|v| v.to_str().ok()).map(str::to_owned)).flatten()))
    }
}

/// A caller who has completed onboarding (has a Profile).
#[derive(Debug, Clone)]
pub struct Ctx {
    pub caller: Caller,
    pub profile: Profile,
    /// An assistant's team membership (title, permissions, assigned clients).
    pub member: Option<Member>,
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
        // An assistant acts only while they're on their head coach's team.
        let member = if profile.role == Role::Assistant {
            let m: Member = state
                .store
                .get(&format!("COACH#{}", profile.coach_id), &format!("ASSISTANT#{}", profile.user_id))
                .await?
                .ok_or(ApiError::Forbidden("you're no longer on this coach's team"))?;
            let head: Option<Profile> = state.store.get(&format!("USER#{}", profile.coach_id), "PROFILE").await?;
            if !head.is_some_and(|h| h.role == Role::Coach && h.suspended_at.is_none()) {
                return Err(ApiError::Forbidden("your coach's account isn't active; contact them or PT Coach support"));
            }
            Some(m)
        } else {
            None
        };
        Ok(Ctx { caller, profile, member })
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

    /// The head coach only (never an assistant): their own id.
    pub fn require_coach(&self) -> ApiResult<&str> {
        if self.is_coach() {
            Ok(&self.profile.user_id)
        } else {
            Err(ApiError::Forbidden("coach only"))
        }
    }

    /// The head coach, or an assistant granted `perm`: the head coach's id,
    /// whose business (partition) the work is done in.
    pub fn require(&self, perm: Perm) -> ApiResult<&str> {
        match &self.member {
            _ if self.is_coach() => Ok(&self.profile.user_id),
            Some(m) if m.can(perm) => Ok(&self.profile.coach_id),
            Some(_) => Err(ApiError::Forbidden("your coach hasn't given you access to this")),
            None => Err(ApiError::Forbidden("coach only")),
        }
    }

    /// The head coach, or an assistant granted any of `perms`.
    pub fn require_any(&self, perms: &[Perm]) -> ApiResult<&str> {
        perms.iter().find_map(|p| self.require(*p).ok()).ok_or(ApiError::Forbidden("your coach hasn't given you access to this"))
    }

    pub fn is_assistant(&self) -> bool {
        self.member.is_some()
    }

    /// Whether this caller may see `client_id` in team-wide lists (queues,
    /// the inbox, the roster): the head coach sees every client, an
    /// assistant only those assigned to them.
    pub fn sees_client(&self, client_id: &str) -> bool {
        self.member.as_ref().is_none_or(|m| m.assigned(client_id))
    }

    /// The name clients see on what this caller sends: an assistant's
    /// carries their title.
    pub fn display_name(&self) -> String {
        self.member.as_ref().map(|m| m.display()).unwrap_or_else(|| self.profile.name.clone())
    }
}

/// Saves a profile the request read earlier, as its own owner edits it.
/// Refused if the account was suspended or deleted in the meantime, so a
/// self-service save can't undo an admin's suspension or bring back a
/// deleted account.
pub async fn save_profile(state: &AppState, p: &Profile) -> ApiResult<()> {
    // The coach it was read with must still be the coach: an admin may have
    // moved the client meanwhile, and this save must not move them back.
    let saved = state
        .store
        .put_if_with(
            &format!("USER#{}", p.user_id),
            "PROFILE",
            p,
            "attribute_exists(pk) AND attribute_not_exists(suspendedAt) AND coachId = :coach",
            &[(":coach", crate::store::s(&p.coach_id))],
        )
        .await?;
    if saved { Ok(()) } else { Err(ApiError::Forbidden("this account was suspended, closed or moved to another coach; reload and try again")) }
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

/// Like `client_scope`, and an assistant too: one granted any of `perms`
/// for a client their head coach has assigned them.
pub async fn client_scope_as(state: &AppState, ctx: &Ctx, client_id: &str, perms: &[Perm]) -> ApiResult<ClientScope> {
    let Some(m) = &ctx.member else { return client_scope(state, ctx, client_id).await };
    if !perms.iter().any(|p| m.can(*p)) {
        return Err(ApiError::Forbidden("your coach hasn't given you access to this"));
    }
    if !m.assigned(client_id) {
        return Err(ApiError::Forbidden("this client isn't assigned to you"));
    }
    let client: Profile = state.store.get(&format!("USER#{client_id}"), "PROFILE").await?.ok_or(ApiError::NotFound("client"))?;
    if client.role == Role::Client && client.coach_id == ctx.profile.coach_id {
        return Ok(ClientScope { client, by_coach: true });
    }
    Err(ApiError::Forbidden("not your client"))
}

/// A client can see only their own data; a coach only their own clients'
/// data. Assistants are refused: routes they may use call `client_scope_as`.
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
