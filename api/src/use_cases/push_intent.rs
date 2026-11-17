use crate::{
    error::ApiError,
    persistence::unix_now,
    push_intents::{PushIntentTarget, repo_config_fingerprint},
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_api_contract::{CreatePushIntentRequest, GitOid};
use scope_domain::{
    error::DomainError,
    repo_actions::reviewed_update_domain_error,
    repo_config::{
        is_repo_config_fingerprint, repo_config_fingerprint as domain_repo_config_fingerprint,
    },
    repository::access::MainPushMode,
    reviewed_updates::config::{ReviewedConfigUpdateInput, apply_reviewed_config_to_repo},
    views::ViewId,
};
use scope_postgres::db::RepositoryMutation;

const MAX_PUSH_INTENT_CONFIG_BYTES: usize = 4096;

pub(crate) struct CreatedPushIntent {
    pub(crate) token: String,
    pub(crate) base_head_oid: Option<String>,
    pub(crate) expires_at_unix: u64,
    pub(crate) lands_as_request: bool,
}

pub(crate) async fn create_push_intent(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    user_id: &str,
    input: CreatePushIntentRequest,
) -> Result<CreatedPushIntent, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .git_push_state(owner, repo_name, user_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    let view: ViewId = input.view.clone().into();
    let views = repo.repo_config.views();
    let mode = repo.access.main_push_mode(repo.lifecycle_state, views);
    if mode == MainPushMode::Denied {
        return Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        )));
    }
    let head_oid = GitOid::try_from(input.head_oid.trim())
        .map(String::from)
        .map_err(|_| ApiError::bad_request("head_oid must be a full SHA-1 Git object id"))?;
    match mode {
        MainPushMode::Denied => Err(ApiError::not_found(format!(
            "repo {owner}/{repo_name} not found"
        ))),
        MainPushMode::FirstPush | MainPushMode::Ready if &view == views.full() => {
            create_canonical_push_intent(state, owner, repo_name, user_id, repo, head_oid, input)
                .await
        }
        MainPushMode::FirstPush | MainPushMode::Ready => Err(ApiError::bad_request(format!(
            "pushes to main through the {} view need the full view's address",
            views.display_name(&view)
        ))),
        MainPushMode::ThroughView(pusher_view) if pusher_view == view => {
            create_view_push_intent(state, owner, repo_name, user_id, head_oid, view).await
        }
        MainPushMode::ThroughView(pusher_view) => Err(ApiError::forbidden(format!(
            "pushes to main from this account go through the {} view",
            views.display_name(&pusher_view)
        ))),
    }
}

async fn create_view_push_intent(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    user_id: &str,
    head_oid: String,
    view: ViewId,
) -> Result<CreatedPushIntent, ApiError> {
    let repo = crate::repo_access::find_read_access(state, owner, repo_name, Some(user_id)).await?;
    let base_head_oid = state
        .metadata
        .repositories()
        .repository_main_oid_for_view(&repo, &view)
        .await?;
    let intent = state.create_push_intent(
        &repo.record.id,
        user_id,
        &head_oid,
        PushIntentTarget::View { view },
    )?;
    Ok(CreatedPushIntent {
        token: intent.token,
        base_head_oid,
        expires_at_unix: intent.expires_at_unix,
        lands_as_request: true,
    })
}

async fn create_canonical_push_intent(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    user_id: &str,
    repo: scope_postgres::db::GitPushState,
    head_oid: String,
    input: CreatePushIntentRequest,
) -> Result<CreatedPushIntent, ApiError> {
    let (Some(input_config), Some(input_base_config_hash)) = (input.config, input.base_config_hash)
    else {
        return Err(ApiError::bad_request(
            "pushes through the full view need the reviewed config and its base hash",
        ));
    };
    let input_config: scope_domain::repo_config::RepoConfig = input_config.try_into()?;
    validate_push_intent_config_transport(&input_config)?;
    let base_config_hash = repo_config_fingerprint(&repo.repo_config)?;
    if !is_repo_config_fingerprint(&input_base_config_hash) {
        return Err(ApiError::bad_request(
            "base_config_hash must be a SHA-256 hex digest",
        ));
    }
    if base_config_hash != input_base_config_hash && repo.repo_config != input_config {
        return Err(ApiError::conflict(
            "repo config changed since review; rerun scope visibility edit",
        ));
    }
    let base_head_oid = repo.git_head.as_ref().map(|head| head.head_oid.clone());
    let base_git_frontier = repo.git_head.as_ref().map(|head| head.frontier());
    let config_changed = repo.repo_config != input_config;
    let applies_config_now = config_changed && base_head_oid.as_deref() == Some(head_oid.as_str());
    if applies_config_now {
        apply_reviewed_config_before_push(
            state,
            owner,
            repo_name,
            &repo,
            user_id,
            input_config.clone(),
        )
        .await?;
    }
    let intent_base_config_hash = if applies_config_now {
        repo_config_fingerprint(&input_config)?
    } else {
        base_config_hash
    };
    let intent = state.create_push_intent(
        &repo.repo_id,
        user_id,
        &head_oid,
        PushIntentTarget::Canonical {
            config: input_config,
            base_config_hash: intent_base_config_hash,
            base_git_frontier,
        },
    )?;

    Ok(CreatedPushIntent {
        token: intent.token,
        base_head_oid,
        expires_at_unix: intent.expires_at_unix,
        lands_as_request: false,
    })
}

async fn apply_reviewed_config_before_push(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    observed: &scope_postgres::db::GitPushState,
    user_id: &str,
    config: scope_domain::repo_config::RepoConfig,
) -> Result<(), ApiError> {
    let expected_config_hash = repo_config_fingerprint(&observed.repo_config)?;
    let expected_git_frontier = observed.git_head.as_ref().map(|head| head.frontier());
    let now = unix_now()?;
    let occurred_at_unix = i64::try_from(now).map_err(ApiError::internal)?;
    let author_id = user_id.to_string();
    let decide =
        move |repo: &mut scope_domain::repository::Repository,
              open_requests_by_view: std::collections::BTreeMap<ViewId, usize>| {
            if repo.push_policy_for_user_id(&author_id).mode != MainPushMode::Ready {
                return Err(DomainError::forbidden("push permission required"));
            }
            let access = repo.access_for_user_id(&author_id);
            if !access.can_change_file_visibility && repo.repo_config != config {
                return Err(DomainError::forbidden(
                    "file visibility permission required",
                ));
            }
            if repo.git_head.as_ref().map(|head| head.frontier()) != expected_git_frontier {
                return Err(DomainError::conflict(
                    "repo content changed since review; rerun scope push --main",
                ));
            }
            if domain_repo_config_fingerprint(&repo.repo_config)
                .map_err(DomainError::invariant_violation)?
                != expected_config_hash
            {
                return Err(DomainError::conflict(
                    "repo config changed since review; rerun scope push --main",
                ));
            }
            let changed = apply_reviewed_config_to_repo(
                repo,
                ReviewedConfigUpdateInput {
                    author_id,
                    config,
                    occurred_at_unix,
                    open_requests_by_view,
                },
            )
            .map_err(reviewed_update_domain_error)?;
            Ok::<_, DomainError>(RepositoryMutation::new(changed))
        };
    let tx = state
        .metadata
        .repositories()
        .begin_repository(owner, repo_name)
        .await?;
    let open_requests_by_view = tx.open_request_counts_by_view().await?;
    let mut repo = tx.repository().clone();
    let mutation = decide(&mut repo, open_requests_by_view)?;
    let changed = tx.save(repo, mutation, now).await?.result;
    if changed {
        let repo = state
            .metadata
            .repositories()
            .git_push_state(owner, repo_name, user_id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
        state.publish_repo_change(
            &repo.incarnation,
            repo.change_version,
            RepoChangeReason::ConfigApplied,
        );
    }
    Ok(())
}

fn validate_push_intent_config_transport(
    config: &scope_domain::repo_config::RepoConfig,
) -> Result<(), ApiError> {
    config.validate().map_err(ApiError::bad_request)?;
    let bytes = serde_json::to_vec(config).map_err(ApiError::internal)?;
    if bytes.len() > MAX_PUSH_INTENT_CONFIG_BYTES {
        return Err(ApiError::bad_request(format!(
            "repo config exceeds {MAX_PUSH_INTENT_CONFIG_BYTES} bytes"
        )));
    }
    Ok(())
}
