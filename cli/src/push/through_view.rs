use super::{
    DEFAULT_SCOPE_BRANCH, ensure_intent_destination, ensure_push_intent_not_expired,
    main_push_denied, main_push_mode, push_head_with_intent, remote_view_mismatch,
};
use crate::{
    api::{ApiSession, CreatePushIntentParams, create_push_intent, get_repo, http_client},
    error::CliError,
    execution::emit,
    git_transport::ScopeRemote,
    login::session_from_cache_or_browser_with_progress,
    progress::PreparationProgress,
    repository_views::{repository_views, summary_repo_config},
};
use scope_domain::{
    repo_config::repo_config_fingerprint, repository::access::MainPushMode,
    requests::main_push_request_name,
};
use serde_json::json;

pub(super) fn push(
    mut progress: PreparationProgress,
    api_url: &str,
    target: &ScopeRemote,
    head_oid: &str,
    wait: bool,
) -> anyhow::Result<()> {
    if wait {
        return Err(CliError::usage(format!(
            "--wait follows workflows on main, but pushes through the {} view land as a request; follow it with scope request show",
            target.view
        ))
        .into());
    }
    progress.set_stage("Verifying login…")?;
    let client = http_client()?;
    let session = session_from_cache_or_browser_with_progress(&client, api_url, &progress)?;
    progress.set_stage("Loading repository access…")?;
    let api = ApiSession::new(&client, api_url, &session.token);
    let repo = get_repo(api, &target.owner, &target.repo)?;
    progress.cancellation().check()?;
    let views = repository_views(&repo.views)?;
    match main_push_mode(&repo.access, repo.lifecycle_state, &views) {
        MainPushMode::ThroughView(view) if view == target.view => {}
        MainPushMode::Denied | MainPushMode::FirstPush => {
            return Err(main_push_denied(target, repo.lifecycle_state));
        }
        MainPushMode::Ready => {
            return Err(remote_view_mismatch(target, &views, views.full()));
        }
        MainPushMode::ThroughView(view) => {
            return Err(remote_view_mismatch(target, &views, &view));
        }
    }
    progress.finish()?;
    let config = summary_repo_config(views.clone());
    let intent = create_push_intent(
        api,
        CreatePushIntentParams {
            owner: &target.owner,
            repo: &target.repo,
            head_oid,
            base_config_hash: &repo_config_fingerprint(&config)?,
            config: &config,
            view: &target.view,
        },
    )?;
    ensure_intent_destination(&intent, true)?;
    ensure_push_intent_not_expired(intent.expires_at_unix)?;
    let view_name = views.display_name(&target.view);
    eprintln!(
        "Publish {}/{} refs/heads/{DEFAULT_SCOPE_BRANCH} at commit {head_oid} through the {view_name} view",
        target.owner, target.repo
    );
    push_head_with_intent(&session.token, target, head_oid, &intent)?;
    let request = main_push_request_name(head_oid);
    emit(
        "push",
        &json!({
            "repository": format!("{}/{}", target.owner, target.repo),
            "remote": target.remote,
            "ref": format!("refs/heads/{DEFAULT_SCOPE_BRANCH}"),
            "commit": head_oid,
            "view": target.view,
            "request": request,
        }),
        vec![format!(
            "Landed as request {request} in the {view_name} view"
        )],
    )
}
