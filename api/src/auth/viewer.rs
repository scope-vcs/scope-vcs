use crate::{auth::scope::optional_scope_user, error::ApiError, state::AppState};
use axum::{
    extract::FromRequestParts,
    http::{HeaderMap, request::Parts},
};
use scope_domain::account::UserAccount;

#[derive(Clone, Debug, Default)]
pub(crate) struct Viewer(Option<UserAccount>);

impl Viewer {
    pub(crate) async fn resolve(state: &AppState, headers: &HeaderMap) -> Result<Self, ApiError> {
        optional_scope_user(state, headers).await.map(Self)
    }

    pub(crate) fn user_id(&self) -> Option<&str> {
        self.0.as_ref().map(|user| user.id.as_str())
    }
}

impl FromRequestParts<AppState> for Viewer {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        Self::resolve(state, &parts.headers).await
    }
}
