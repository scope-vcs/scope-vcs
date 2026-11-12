use crate::{
    auth::clerk::bearer_token, clerk_backend::ClerkOAuthTokenCheck, error::ApiError,
    state::AppState,
};
use axum::http::HeaderMap;
use scope_domain::account::{
    UserAccount,
    agent_access::{AgentGrant, AgentScope},
};

pub(crate) struct AgentCaller {
    pub(crate) user: UserAccount,
    grant: AgentGrant,
}

impl AgentCaller {
    pub(crate) fn allows(&self, scope: AgentScope) -> bool {
        self.grant.allows(scope)
    }
}

pub(crate) async fn require_agent_caller(
    state: &AppState,
    headers: &HeaderMap,
    resource: &str,
) -> Result<AgentCaller, ApiError> {
    let token =
        bearer_token(headers)?.ok_or_else(|| ApiError::unauthorized("agent sign-in required"))?;
    let token = match state.clerk_backend.verify_oauth_access_token(token).await {
        ClerkOAuthTokenCheck::Active(token) => token,
        ClerkOAuthTokenCheck::Inactive => {
            return Err(ApiError::unauthorized(
                "agent token is invalid, expired, or revoked",
            ));
        }
        ClerkOAuthTokenCheck::Unavailable(diagnostic) => {
            return Err(ApiError::infrastructure_unavailable(format!(
                "verify agent token: {diagnostic}"
            )));
        }
    };
    if !token.audiences.iter().any(|audience| audience == resource) {
        return Err(ApiError::unauthorized(
            "agent token was issued for another resource",
        ));
    }
    let user = state
        .metadata
        .auth()
        .clerk_subject_user(&token.subject)
        .await?
        .ok_or_else(|| {
            ApiError::unauthorized("sign in to Scope in a browser before connecting an agent")
        })?;
    Ok(AgentCaller {
        user,
        grant: AgentGrant::from_keys(token.scopes.iter().map(String::as_str)),
    })
}
