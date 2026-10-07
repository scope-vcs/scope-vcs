use super::{
    DEFAULT_SCOPE_BRANCH, ensure_intent_destination, ensure_push_intent_not_expired,
    main_push_denied, main_push_mode, push_head_with_intent, remote_view_mismatch,
};
use crate::{
    api::{
        ApiSession, CreatePushIntentParams, RequestListItemResponse, create_push_intent, get_repo,
        http_client, list_requests,
    },
    error::CliError,
    execution::emit,
    git_transport::ScopeRemote,
    login::session_from_cache_or_browser_with_progress,
    progress::PreparationProgress,
    repository_views::repository_views,
};
use scope_domain::{repository::access::MainPushMode, requests::main_push_request_names};
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
    let intent = create_push_intent(
        api,
        CreatePushIntentParams {
            owner: &target.owner,
            repo: &target.repo,
            head_oid,
            base_config_hash: None,
            config: None,
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
    let request = landed_request(api, target, &session.user.id, head_oid)?;
    emit(
        "push",
        &json!({
            "repository": format!("{}/{}", target.owner, target.repo),
            "remote": target.remote,
            "ref": format!("refs/heads/{DEFAULT_SCOPE_BRANCH}"),
            "commit": head_oid,
            "view": target.view,
            "landed": true,
            "request": {"id": request.id, "name": request.name},
        }),
        vec![format!(
            "Landed as request {} ({}) in the {view_name} view",
            request.name, request.id
        )],
    )
}

fn landed_request(
    api: ApiSession<'_>,
    target: &ScopeRemote,
    pusher_user_id: &str,
    head_oid: &str,
) -> anyhow::Result<RequestListItemResponse> {
    let names = main_push_request_names(head_oid).collect::<Vec<_>>();
    let mut newest: Option<(usize, RequestListItemResponse)> = None;
    let mut cursor = None;
    loop {
        let page = list_requests(api, &target.owner, &target.repo, cursor.as_deref())?;
        for request in page.requests {
            let generated = names.iter().position(|name| *name == request.name);
            let Some(position) = generated.filter(|_| {
                request.author_user_id.as_deref() == Some(pusher_user_id)
                    && request.head_oid.as_str() == head_oid
            }) else {
                continue;
            };
            if newest.as_ref().is_none_or(|(latest, _)| position > *latest) {
                newest = Some((position, request));
            }
        }
        match page.next_cursor {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    newest.map(|(_, request)| request).ok_or_else(|| {
        CliError::partial(
            format!(
                "the push to {}/{} landed, but Scope lists no main push request for {head_oid}; run scope request list",
                target.owner, target.repo
            ),
            json!({"commit": head_oid, "view": target.view, "landed": true}),
        )
        .into()
    })
}
