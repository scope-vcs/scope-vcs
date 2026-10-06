use super::{ReceivePackAccess, request_ref};
use crate::{
    config::GIT_PUSH_TOKEN_PREFIX,
    error::ApiError,
    git::{
        InitialPushCredential, ReceivePackAuthorization, authorize_git_push_token_for_repo,
        authorize_initial_push_for_repo, git_credential_error,
        git_push_credentials_after_scope_token, invalid_git_credentials,
    },
    push_intents::ValidatedPushIntent,
    state::AppState,
};
use scope_domain::{
    repository::{RepoLifecycleState, access::MainPushMode},
    views::ViewId,
};
use scope_postgres::db::{GitPushContext, GitPushCredentials};

pub(crate) async fn authorize(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
    authorization: ReceivePackAuthorization,
    push_intent_secret: Option<&str>,
) -> Result<ReceivePackAccess, ApiError> {
    match authorization {
        ReceivePackAuthorization::ScopeToken { secret } => {
            let push_intent = required_push_intent(state, push_intent_secret)?;
            let repo = git_push_credentials_after_scope_token(state, owner, repo_name).await?;
            let views = state
                .metadata
                .repositories()
                .repository_access(owner, repo_name, None)
                .await?
                .ok_or_else(invalid_git_credentials)?
                .views;
            if view != views.full() {
                return Err(ApiError::forbidden(format!(
                    "Git push tokens push through the {} view's address",
                    views.display_name(views.full())
                )));
            }
            authorize_scope_token(repo, secret, push_intent)
        }
        ReceivePackAuthorization::ScopeUser(user) => {
            if let Some(secret) = push_intent_secret
                && let Ok(push_intent) = state.validate_push_intent_secret(secret)
                && let Some(context) = state
                    .metadata
                    .repositories()
                    .git_push_context(owner, repo_name, &user.id)
                    .await?
                && let Some(access) = main_push_access(&context, view, &user.id, &push_intent)?
            {
                return Ok(access);
            }
            authorize_scope_user(state, owner, repo_name, view, user.id, push_intent_secret).await
        }
    }
}

fn authorize_scope_token(
    repo: GitPushCredentials,
    secret: String,
    push_intent: ValidatedPushIntent,
) -> Result<ReceivePackAccess, ApiError> {
    let credential = if secret.starts_with(GIT_PUSH_TOKEN_PREFIX) {
        InitialPushCredential::GitPushToken { secret }
    } else {
        InitialPushCredential::FirstPushToken { secret }
    };
    match repo.record.lifecycle_state {
        RepoLifecycleState::AwaitingFirstPush => {
            authorize_initial_push_for_repo(&repo, &credential).map_err(git_credential_error)?;
            let author_id = repo.record.owner_user_id.clone();
            push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
            Ok(ReceivePackAccess::FirstPush {
                author_id,
                incarnation: repo.record.incarnation(),
                push_intent,
            })
        }
        RepoLifecycleState::Ready => match credential {
            InitialPushCredential::GitPushToken { secret } => {
                let author_id = authorize_git_push_token_for_repo(&repo, &secret)
                    .map_err(git_credential_error)?;
                push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                Ok(ReceivePackAccess::ReadyMember {
                    author_id,
                    incarnation: repo.record.incarnation(),
                    push_intent,
                })
            }
            InitialPushCredential::FirstPushToken { .. } => Err(invalid_git_credentials()),
        },
    }
}

fn main_push_access(
    context: &GitPushContext,
    view: &ViewId,
    author_id: &str,
    push_intent: &ValidatedPushIntent,
) -> Result<Option<ReceivePackAccess>, ApiError> {
    let views = context.repo_config.views();
    let access = match context
        .access
        .main_push_mode(context.lifecycle_state, views)
    {
        MainPushMode::Ready if view == views.full() && push_intent.through_view().is_none() => {
            ReceivePackAccess::ReadyMember {
                author_id: author_id.to_string(),
                incarnation: context.incarnation.clone(),
                push_intent: push_intent.clone(),
            }
        }
        MainPushMode::ThroughView(pusher_view)
            if &pusher_view == view && push_intent.through_view() == Some(view) =>
        {
            ReceivePackAccess::ViewMainPusher {
                author_id: author_id.to_string(),
                incarnation: context.incarnation.clone(),
                view: pusher_view,
                push_intent: push_intent.clone(),
            }
        }
        MainPushMode::Denied
        | MainPushMode::FirstPush
        | MainPushMode::Ready
        | MainPushMode::ThroughView(_) => return Ok(None),
    };
    push_intent.ensure_repo_user(&context.repo_id, author_id)?;
    Ok(Some(access))
}

async fn authorize_scope_user(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
    author_id: String,
    push_intent_secret: Option<&str>,
) -> Result<ReceivePackAccess, ApiError> {
    let not_found = || ApiError::not_found(format!("repo {owner}/{repo_name} not found"));
    let context = state
        .metadata
        .repositories()
        .repository_read_access(owner, repo_name, Some(&author_id))
        .await?
        .ok_or_else(not_found)?;
    let repo = &context.record;
    let access = context.access;
    let views = &context.views;
    let push_mode = access.main_push_mode(repo.lifecycle_state, views);
    if push_mode == MainPushMode::FirstPush && view == views.full() {
        let push_intent = required_push_intent(state, push_intent_secret)?;
        push_intent.ensure_repo_user(&repo.id, &author_id)?;
        return Ok(ReceivePackAccess::FirstPush {
            author_id,
            incarnation: repo.incarnation(),
            push_intent,
        });
    }
    if !access.can_read_view(views, view) {
        return Err(not_found());
    }
    let rejection = match push_mode {
        MainPushMode::Denied => not_found(),
        _ if repo.lifecycle_state == RepoLifecycleState::AwaitingFirstPush => {
            return Err(ApiError::conflict(
                "repo is awaiting its first push and cannot receive another push",
            ));
        }
        _ => match push_intent_secret.map(|secret| state.validate_push_intent_secret(secret)) {
            Some(Ok(_)) => ApiError::forbidden(format!(
                "this Scope push intent does not push to main through the {} view",
                views.display_name(view)
            )),
            Some(Err(error)) => error,
            None => ApiError::forbidden("valid Scope push intent required"),
        },
    };
    if repo.lifecycle_state == RepoLifecycleState::Ready
        && request_ref::actor_has_open_editable_request(
            state, &repo.id, &author_id, access, views, view,
        )
        .await?
    {
        Ok(ReceivePackAccess::RequestContributor {
            author_id,
            incarnation: repo.incarnation(),
            view: view.clone(),
        })
    } else {
        Err(rejection)
    }
}

fn required_push_intent(
    state: &AppState,
    secret: Option<&str>,
) -> Result<ValidatedPushIntent, ApiError> {
    let secret = secret.ok_or_else(|| ApiError::forbidden("valid Scope push intent required"))?;
    state.validate_push_intent_secret(secret)
}
