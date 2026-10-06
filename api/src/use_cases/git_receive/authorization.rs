use super::{ReceivePackAccess, request_ref};
use crate::{
    auth::scope::principal_for_user_id,
    config::GIT_PUSH_TOKEN_PREFIX,
    error::ApiError,
    git::{
        InitialPushCredential, ReceivePackAuthorization, authorize_git_push_token_for_repo,
        authorize_initial_push_for_repo, find_repo_after_git_scope_token, git_credential_error,
        invalid_git_credentials,
    },
    push_intents::ValidatedPushIntent,
    repo_access::{ensure_repo_read, find_repo},
    state::AppState,
};
use scope_domain::{
    repository::{RepoLifecycleState, Repository, access::MainPushMode},
    views::ViewId,
};
use scope_postgres::db::GitPushContext;

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
            let repo = find_repo_after_git_scope_token(state, owner, repo_name).await?;
            let views = repo.repo_config.views();
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
    repo: Repository,
    secret: String,
    push_intent: ValidatedPushIntent,
) -> Result<ReceivePackAccess, ApiError> {
    let credential = if secret.starts_with(GIT_PUSH_TOKEN_PREFIX) {
        InitialPushCredential::GitPushToken { secret }
    } else {
        InitialPushCredential::FirstPushToken { secret }
    };
    if repo.is_waiting_for_first_push() {
        authorize_initial_push_for_repo(&repo, &credential).map_err(git_credential_error)?;
        let author_id = repo.record.owner_user_id.clone();
        push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
        return Ok(ReceivePackAccess::FirstPush {
            author_id,
            incarnation: repo.incarnation(),
            push_intent,
        });
    }
    match repo.record.lifecycle_state {
        RepoLifecycleState::AwaitingFirstPush => Err(ApiError::conflict(
            "repo is awaiting its first push and cannot receive another push",
        )),
        RepoLifecycleState::Ready => match credential {
            InitialPushCredential::GitPushToken { secret } => {
                let author_id = authorize_git_push_token_for_repo(&repo, &secret)
                    .map_err(git_credential_error)?;
                push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
                Ok(ReceivePackAccess::ReadyMember {
                    author_id,
                    incarnation: repo.incarnation(),
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
    let repo = find_repo(state, owner, repo_name).await?;
    let principal = principal_for_user_id(&repo, &author_id);
    let push_policy = repo.push_policy_for_user_id(&author_id);
    let views = repo.repo_config.views();
    if push_policy.mode == MainPushMode::FirstPush && view == views.full() {
        let push_intent = required_push_intent(state, push_intent_secret)?;
        push_intent.ensure_repo_user(&repo.record.id, &author_id)?;
        return Ok(ReceivePackAccess::FirstPush {
            author_id,
            incarnation: repo.incarnation(),
            push_intent,
        });
    }
    let not_found = || ApiError::not_found(format!("repo {owner}/{repo_name} not found"));
    if ensure_repo_read(&repo, &principal).is_err()
        || !repo.can_read_view(&push_policy.access, view)
    {
        return Err(not_found());
    }
    let rejection = match push_policy.mode {
        MainPushMode::Denied => not_found(),
        _ if repo.record.lifecycle_state == RepoLifecycleState::AwaitingFirstPush => {
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
    if repo.record.lifecycle_state == RepoLifecycleState::Ready
        && request_ref::actor_has_open_editable_request(
            state,
            &repo,
            &author_id,
            push_policy.access,
            view,
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
