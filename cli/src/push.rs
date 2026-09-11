use crate::api::ApiSession;
use crate::{
    agent_context::ensure_repo_rules_ready_for_push,
    api::{
        CreatePushIntentParams, PushTriggerEvaluationResponse, RepoLifecycleState,
        RepositoryAccessResponse, RepositoryActor, api_url, create_push_intent,
        get_push_trigger_evaluation, get_repo_config, http_client,
    },
    git_repo::{
        GitRepo, changed_paths_since_scope_base_at_commit, ensure_git_repo_ready,
        fetch_scope_remote_with_bearer, git_remote_push_url, head_oid, mark_scope_remote_pushed,
        push_head_with_bearer, scope_git_origin, scope_remote_head_oid, warn_if_dirty_working_tree,
    },
    git_transport::{GitAccess, ScopeRemote, select_scope_push_remote},
    login::session_from_cache_or_browser,
    repo_config::{
        ensure_scope_repo_config_exists, load_worktree_scope_repo_config,
        load_worktree_scope_repo_config_base_hash, mark_worktree_scope_repo_config_synced,
        repo_config_path, write_worktree_scope_repo_config_with_base,
    },
    review::{ensure_review_terminal_available, run_push_review},
};
use crate::{
    error::CliError,
    execution::{emit, json as json_output},
};
use anyhow::bail;
use scope_api_contract::{ErrorCode, ErrorResponse, PushTriggerEvaluationState};
use scope_domain::repo_config::repo_config_fingerprint;
use serde_json::json;
use std::{
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const DEFAULT_SCOPE_BRANCH: &str = "main";
const PUSH_EVALUATION_MAX_POLLS: usize = 300;

#[derive(Debug, Eq, PartialEq)]
pub struct ScopePushOutcome {
    pub owner: String,
    pub repo: String,
}

pub fn run(explicit_remote: Option<&str>, no_review: bool, wait: bool) -> anyhow::Result<()> {
    let git_repo = ensure_git_repo_ready("scope push")?;
    let reviewed_head_oid = head_oid(&git_repo)?;
    ensure_repo_rules_ready_for_push(&git_repo.root, &reviewed_head_oid)?;
    let config_created = ensure_scope_repo_config_exists(&git_repo.root)?;
    let config_path = repo_config_path(&git_repo.root)?;
    let mut config = load_worktree_scope_repo_config(&git_repo.root)?;
    warn_if_dirty_working_tree(&git_repo)?;
    if !no_review {
        ensure_review_terminal_available("scope push review")?;
    }

    let api_url = api_url()?;
    let remote = select_scope_push_remote(&git_repo, &api_url, explicit_remote)?;
    let target = load_scope_remote(&git_repo, &api_url, &remote)?;
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    let api = ApiSession::new(&client, &api_url, &session.token);
    let push_context = get_repo_config(api, &target.owner, &target.repo)?;
    ensure_scope_remote_can_receive_push(
        &target,
        push_context.lifecycle_state,
        &push_context.access,
    )?;
    if config_created {
        write_worktree_scope_repo_config_with_base(&git_repo.root, &push_context.config)?;
        config = push_context.config.clone();
        eprintln!("Created {}", config_path.display());
    } else {
        let local_config_hash = repo_config_fingerprint(&config)?;
        match load_worktree_scope_repo_config_base_hash(&git_repo.root) {
            Ok(hash) if hash == push_context.config_hash => {}
            Ok(_) if local_config_hash == push_context.config_hash => {
                mark_worktree_scope_repo_config_synced(&git_repo.root, &config)?;
            }
            Ok(hash) if hash == local_config_hash => {
                write_worktree_scope_repo_config_with_base(&git_repo.root, &push_context.config)?;
                config = push_context.config.clone();
                eprintln!(
                    "Scope repo config changed; refreshed {}",
                    config_path.display()
                );
            }
            Ok(_) => return Err(CliError::new(ErrorResponse::new(ErrorCode::Conflict, format!(
                "Scope repo config changed, and local {} has unsynced edits. Run scope visibility edit, resolve the config, then retry scope push --main.", config_path.display()
            ))).into()),
            Err(_) if local_config_hash == push_context.config_hash => {
                mark_worktree_scope_repo_config_synced(&git_repo.root, &config)?;
            }
            Err(error) => return Err(CliError::new(ErrorResponse::new(ErrorCode::Conflict, format!(
                "{error}. Local {} has unsynced edits, so Scope will not overwrite it.", config_path.display()
            ))).into()),
        }
    }
    let local_remote_head = scope_remote_head_oid(&git_repo, &remote, DEFAULT_SCOPE_BRANCH)?;
    if push_context.lifecycle_state == RepoLifecycleState::Ready
        && local_remote_head.as_deref() != push_context.head_oid.as_deref()
    {
        fetch_scope_remote_with_bearer(
            &git_repo,
            &target.permissioned_url,
            &remote,
            DEFAULT_SCOPE_BRANCH,
            &session.token,
        )?;
    }
    let reviewed_base_oid = if no_review {
        None
    } else if push_context.lifecycle_state == RepoLifecycleState::Ready {
        Some(scope_remote_head_oid(
            &git_repo,
            &remote,
            DEFAULT_SCOPE_BRANCH,
        )?)
    } else {
        Some(None)
    };
    if let Some(review_base_oid) = &reviewed_base_oid {
        let changed_paths = changed_paths_since_scope_base_at_commit(
            &git_repo,
            review_base_oid.as_deref(),
            &reviewed_head_oid,
        )?;
        config = run_push_review(&git_repo, &reviewed_head_oid, &changed_paths)?;
    }
    let base_config_hash = load_worktree_scope_repo_config_base_hash(&git_repo.root)?;
    let intent = create_push_intent(
        api,
        CreatePushIntentParams {
            owner: &target.owner,
            repo: &target.repo,
            head_oid: &reviewed_head_oid,
            base_config_hash: &base_config_hash,
            config: &config,
        },
    )?;
    if let Some(review_base_oid) = &reviewed_base_oid {
        ensure_reviewed_base_matches_intent(
            review_base_oid.as_deref(),
            intent.base_head_oid.as_deref(),
        )?;
    }
    ensure_review_base_matches_intent(
        &git_repo,
        &target.permissioned_url,
        &remote,
        &session.token,
        intent.base_head_oid.as_deref(),
    )?;
    ensure_push_intent_not_expired(intent.expires_at_unix)?;
    eprintln!(
        "Publish {}/{} refs/heads/{} at commit {}",
        target.owner, target.repo, DEFAULT_SCOPE_BRANCH, reviewed_head_oid
    );

    let outcome = match push_reviewed_head_with_intent(
        &session.token,
        &target,
        &reviewed_head_oid,
        &intent.token,
    ) {
        Ok(outcome) => outcome,
        Err(_) if push_intent_expired(intent.expires_at_unix) => {
            return Err(CliError::new(ErrorResponse::new(
                ErrorCode::Conflict,
                "Scope push review expired; rerun scope push --main",
            ))
            .into());
        }
        Err(error) => return Err(error),
    };
    let mut receipt = json!({"repository": format!("{}/{}", outcome.owner, outcome.repo), "remote": remote, "ref": format!("refs/heads/{DEFAULT_SCOPE_BRANCH}"), "commit": reviewed_head_oid, "applied": true, "tracking_updated": false, "config_synced": false});
    mark_scope_remote_pushed(&git_repo, &remote, DEFAULT_SCOPE_BRANCH, &reviewed_head_oid)
        .map_err(|error| applied_push_error(&receipt, format!("Push applied, but local tracking setup failed: {error:#}"), "Keep this commit. Fix the reported local Git error, then run scope pull before publishing again."))?;
    receipt["tracking_updated"] = json!(true);
    mark_worktree_scope_repo_config_synced(&git_repo.root, &config)
        .map_err(|error| applied_push_error(&receipt, format!("Push applied, but saving local visibility configuration failed: {error:#}"), "Keep this commit. Fix the reported local filesystem error, then run scope visibility show before publishing again."))?;
    receipt["config_synced"] = json!(true);
    if wait {
        eprintln!("Push applied at {reviewed_head_oid}; waiting for workflows.");
        let wait_result =
            get_push_trigger_evaluation(api, &target.owner, &target.repo, &reviewed_head_oid)
                .and_then(|evaluation| wait_for_push_runs(api, &target, &remote, evaluation));
        receipt["workflows"] = wait_result.map_err(|error| applied_push_error(&receipt,
            format!("Push applied, but workflow waiting failed: {error:#}"),
            "Inspect scope run list and scope run show for this commit. Do not repeat the push to retry waiting."))?;
    }
    emit(
        "push",
        &receipt,
        vec![format!(
            "Pushed to Scope: {}/{}\nPush applied by Scope.",
            outcome.owner, outcome.repo
        )],
    )
}

fn wait_for_push_runs(
    api: ApiSession<'_>,
    target: &ScopeRemote,
    remote: &str,
    mut evaluation: PushTriggerEvaluationResponse,
) -> anyhow::Result<serde_json::Value> {
    let mut polls = 0;
    while evaluation.state == PushTriggerEvaluationState::Pending {
        ensure_evaluation_poll_remaining(polls)?;
        thread::sleep(Duration::from_secs(1));
        evaluation =
            get_push_trigger_evaluation(api, &target.owner, &target.repo, &evaluation.head_oid)?;
        polls += 1;
    }
    match evaluation.state {
        PushTriggerEvaluationState::ConfigurationError | PushTriggerEvaluationState::Failed => {
            bail!(
                "push workflow evaluation failed: {}",
                evaluation
                    .message
                    .as_deref()
                    .unwrap_or("unknown configuration error")
            )
        }
        PushTriggerEvaluationState::Pending => unreachable!("pending state was polled above"),
        PushTriggerEvaluationState::Succeeded => {}
    }
    if evaluation.checks.is_empty() {
        eprintln!("No main-push workflows matched.");
        return Ok(json!([]));
    }
    let mut failures = Vec::new();
    let mut completed = Vec::new();
    for check in evaluation.checks {
        eprintln!("Queued {} · {}", check.workflow_name, check.run.id);
        let result = if json_output() {
            crate::run::wait_completion(&check.run.id, Some(remote)).map(|run| json!(run))
        } else {
            crate::run::watch(&check.run.id, Some(remote))
                .map(|_| json!({"id": check.run.id, "workflow_name": check.workflow_name}))
        };
        match result {
            Ok(run) => completed.push(run),
            Err(error) => failures.push(error.to_string()),
        }
    }
    if failures.is_empty() {
        Ok(json!(completed))
    } else {
        bail!("push workflows failed: {}", failures.join("; "))
    }
}

fn applied_push_error(receipt: &serde_json::Value, message: String, recovery: &str) -> CliError {
    let mut receipt = receipt.clone();
    receipt["operation"] = json!("push");
    receipt["recovery"] = json!(recovery);
    CliError::partial(message, receipt)
}

fn ensure_evaluation_poll_remaining(polls: usize) -> anyhow::Result<()> {
    if polls >= PUSH_EVALUATION_MAX_POLLS {
        bail!(
            "timed out waiting for Scope to evaluate push workflows; the push was already applied"
        );
    }
    Ok(())
}

fn ensure_push_intent_not_expired(expires_at_unix: u64) -> anyhow::Result<()> {
    if push_intent_expired(expires_at_unix) {
        return Err(CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "Scope push review expired; rerun scope push --main",
        ))
        .into());
    }
    Ok(())
}

fn push_intent_expired(expires_at_unix: u64) -> bool {
    unix_now() >= expires_at_unix
}

fn ensure_review_base_matches_intent(
    git_repo: &GitRepo,
    push_url: &str,
    remote: &str,
    session_token: &str,
    intent_base_head_oid: Option<&str>,
) -> anyhow::Result<()> {
    let Some(intent_base_head_oid) = intent_base_head_oid else {
        return Ok(());
    };
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }

    fetch_scope_remote_with_bearer(
        git_repo,
        push_url,
        remote,
        DEFAULT_SCOPE_BRANCH,
        session_token,
    )?;
    if scope_remote_head_oid(git_repo, remote, DEFAULT_SCOPE_BRANCH)?.as_deref()
        == Some(intent_base_head_oid)
    {
        return Ok(());
    }
    Err(CliError::new(ErrorResponse::new(
        ErrorCode::Conflict,
        "Scope changed while preparing push review; rerun scope push --main",
    ))
    .into())
}

fn ensure_reviewed_base_matches_intent(
    reviewed_base_oid: Option<&str>,
    intent_base_head_oid: Option<&str>,
) -> anyhow::Result<()> {
    if reviewed_base_oid != intent_base_head_oid {
        return Err(CliError::new(ErrorResponse::new(
            ErrorCode::Conflict,
            "Scope changed while preparing push review; rerun scope push --main",
        ))
        .into());
    }
    Ok(())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn load_scope_remote(
    git_repo: &GitRepo,
    api_url: &str,
    remote: &str,
) -> anyhow::Result<ScopeRemote> {
    let push_url = git_remote_push_url(git_repo, remote)?;
    let git_origin = scope_git_origin(git_repo, api_url)?;
    let target = ScopeRemote::parse(&git_origin, remote, &push_url)?;
    if target.access != GitAccess::Permissioned {
        return Err(
            CliError::usage("Scope remote must have path /git/permissioned/owner/repo").into(),
        );
    }
    Ok(target)
}

pub fn ensure_scope_remote_can_receive_push(
    target: &ScopeRemote,
    lifecycle_state: RepoLifecycleState,
    access: &RepositoryAccessResponse,
) -> anyhow::Result<()> {
    if lifecycle_state == RepoLifecycleState::AwaitingFirstPush {
        ensure_awaiting_first_push_repo_can_receive_first_push(
            &target.owner,
            &target.repo,
            access.actor,
        )
    } else {
        ensure_ready_repo_can_receive_push(
            &target.owner,
            &target.repo,
            lifecycle_state,
            access.can_push,
        )
    }
}

pub fn push_reviewed_head_with_intent(
    session_token: &str,
    target: &ScopeRemote,
    reviewed_head_oid: &str,
    push_intent_token: &str,
) -> anyhow::Result<ScopePushOutcome> {
    push_head_with_bearer(
        &target.permissioned_url,
        reviewed_head_oid,
        DEFAULT_SCOPE_BRANCH,
        session_token,
        push_intent_token,
    )?;

    Ok(ScopePushOutcome {
        owner: target.owner.clone(),
        repo: target.repo.clone(),
    })
}

fn ensure_awaiting_first_push_repo_can_receive_first_push(
    owner: &str,
    repo: &str,
    actor: RepositoryActor,
) -> anyhow::Result<()> {
    if actor != RepositoryActor::Owner {
        return Err(CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            format!("you do not have owner access to first-push {owner}/{repo}"),
        ))
        .into());
    }
    Ok(())
}

fn ensure_ready_repo_can_receive_push(
    owner: &str,
    repo: &str,
    lifecycle_state: RepoLifecycleState,
    can_push: bool,
) -> anyhow::Result<()> {
    match lifecycle_state {
        RepoLifecycleState::AwaitingFirstPush => {
            bail!("repo {owner}/{repo} is waiting for its first push. Run: scope init");
        }
        RepoLifecycleState::Ready => {}
    }

    if !can_push {
        return Err(CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            format!("you do not have write access to {owner}/{repo}"),
        ))
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_push_intent_reports_rerun_message() {
        let error = ensure_push_intent_not_expired(0).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 5);
        assert!(
            error
                .to_string()
                .contains("Scope push review expired; rerun scope push --main")
        );
        ensure_push_intent_not_expired(unix_now().saturating_add(60)).unwrap();
    }

    #[test]
    fn push_evaluation_wait_is_bounded() {
        ensure_evaluation_poll_remaining(PUSH_EVALUATION_MAX_POLLS - 1).unwrap();
        let error = ensure_evaluation_poll_remaining(PUSH_EVALUATION_MAX_POLLS).unwrap_err();
        assert!(error.to_string().contains("the push was already applied"));
    }

    #[test]
    fn reviewed_base_must_match_push_intent_base() {
        ensure_reviewed_base_matches_intent(None, None).unwrap();
        ensure_reviewed_base_matches_intent(Some("abc"), Some("abc")).unwrap();
        let error = ensure_reviewed_base_matches_intent(Some("abc"), Some("def")).unwrap_err();
        assert_eq!(crate::error::exit_code(&error), 5);
        assert!(
            error
                .to_string()
                .contains("Scope changed while preparing push review; rerun scope push --main")
        );
        assert!(ensure_reviewed_base_matches_intent(None, Some("def")).is_err());
        assert!(ensure_reviewed_base_matches_intent(Some("abc"), None).is_err());
    }

    #[test]
    fn first_push_requires_owner_access() {
        ensure_awaiting_first_push_repo_can_receive_first_push(
            "owner",
            "repo",
            RepositoryActor::Owner,
        )
        .unwrap();
        for actor in [RepositoryActor::Member, RepositoryActor::Public] {
            assert!(
                ensure_awaiting_first_push_repo_can_receive_first_push("owner", "repo", actor)
                    .is_err()
            );
        }
    }

    #[test]
    fn published_push_requires_write_access() {
        for (state, can_push, allowed) in [
            (RepoLifecycleState::Ready, true, true),
            (RepoLifecycleState::Ready, false, false),
            (RepoLifecycleState::AwaitingFirstPush, true, false),
        ] {
            assert_eq!(
                ensure_ready_repo_can_receive_push("owner", "repo", state, can_push).is_ok(),
                allowed,
            );
        }
    }
}
