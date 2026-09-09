use super::*;
use crate::api::ApiSession;

pub(super) fn start_request_branch(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    args: RequestStartArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(Some(git_repo), api, args.remote.as_deref())?;
    local::require_git_remote(&context)?;
    let audience = start_audience(context.repo.access.actor, args.audience)?;
    let base_oid = refresh_main_projection(git_repo, &context.target, audience, api.token)?;
    let branch = args.name.trim().to_string();
    scope_domain::requests::validate_request_name(&branch)
        .map_err(|error| anyhow::anyhow!(error.message))?;
    let local_ref = format!("refs/heads/{branch}");
    if try_run_git_in_repo(git_repo, &["show-ref", "--verify", "--quiet", &local_ref])? {
        bail!("local branch '{branch}' already exists");
    }
    let remote_main = remote_main_ref(&context.target.remote);
    let response = api_start_request(
        api,
        StartRequestParams {
            owner: &context.target.owner,
            repo: &context.target.repo,
            name: branch.clone(),
            title: args.title,
            audience,
        },
    )?;
    if let Err(switch_error) = run_git_in_repo(
        git_repo,
        &[
            "switch",
            "--quiet",
            "--no-track",
            "-c",
            &branch,
            &remote_main,
        ],
    ) {
        let cleanup = api_close_request(
            api,
            &context.target.owner,
            &context.target.repo,
            &response.request.id,
        );
        return match cleanup {
            Ok(_) => Err(switch_error).context(
                "create local request branch failed; the empty request was closed and removed, so it is safe to retry",
            ),
            Err(cleanup_error) => Err(crate::error::CliError::partial(
                format!("request {} was created, but local branch creation failed ({switch_error}) and cleanup failed ({cleanup_error}); close it with `scope request close --remote {} --request {} --yes` before retrying start", response.request.id, context.target.remote, response.request.id),
                serde_json::json!({
                    "repository": format!("{}/{}", context.target.owner, context.target.repo),
                    "request_id": response.request.id,
                    "failed_step": "create_local_branch",
                    "retry_command": ["scope", "request", "close", "--remote", context.target.remote, "--request", response.request.id, "--yes"],
                }),
            ).into()),
        };
    }
    let recover = |stage, error| {
        recovery::request_partial(&context, &response.request, &branch, stage, false, error)
    };
    store_request_metadata(git_repo, &branch, &context, &response.request)
        .map_err(|error| recover("save_local_metadata", error))?;
    let request_head_oid = head_oid(git_repo).map_err(|error| recover("read_local_head", error))?;
    push_request_head(
        &context.target,
        api.token,
        &request_head_oid,
        &response.request.id,
        &response.request.name,
    )
    .map_err(|error| recover("push_request_head", error))?;
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &response.request.name,
        &request_head_oid,
    )
    .map_err(|error| {
        recovery::request_partial(
            &context,
            &response.request,
            &branch,
            "configure_tracking",
            true,
            error,
        )
    })?;

    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend([
        format!(
            "Started request {} ({}) on branch {branch} from {} ({})",
            response.request.name,
            response.request.id,
            projection_label_for_audience(audience),
            short_oid(&base_oid)
        ),
        "Next: commit changes, then run scope request push".to_string(),
        format!(
            "Remote: {}/{}",
            context.target.remote, response.request.name
        ),
        "Useful while working: scope pull, scope request status".to_string(),
    ]);
    let result = StartResult {
        repo: context.repo,
        request: response.request,
        branch,
        base_oid,
        remote: context.target.remote,
    };
    Ok(RequestCommandOutcome::new(
        "request.start",
        RequestCommandResult::Started(result),
        human_lines,
    ))
}

pub(super) fn push_request_branch(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    remote: Option<String>,
    request_id: Option<String>,
    machine_output: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    if !machine_output {
        warn_if_dirty_working_tree(git_repo)?;
    }
    let context = load_context(Some(git_repo), api, remote.as_deref())?;
    local::require_git_remote(&context)?;
    let request_id = request_id_for_context(Some(git_repo), api, &context, request_id)?;
    let detail = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )?;
    if !detail.request.permissions.can_push_branch {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            format!(
                "request {} cannot be pushed by this user",
                detail.request.id
            ),
        ))
        .into());
    }
    let branch = current_branch(git_repo)?;
    let request_head_oid = head_oid(git_repo)?;
    let current_main_oid = refresh_main_projection(
        git_repo,
        &context.target,
        detail.request.audience,
        api.token,
    )?;
    ensure_public_request_paths_allowed(git_repo, &detail, &current_main_oid, &request_head_oid)?;
    push_request_head(
        &context.target,
        api.token,
        &request_head_oid,
        &detail.request.id,
        &detail.request.name,
    )
    .map_err(|error| {
        recovery::request_partial(
            &context,
            &detail.request,
            &branch,
            "push_request_head",
            false,
            error,
        )
    })?;
    let recover = |stage, error| {
        recovery::request_partial(&context, &detail.request, &branch, stage, true, error)
    };
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &detail.request.name,
        &request_head_oid,
    )
    .map_err(|error| recover("configure_tracking", error))?;
    store_request_metadata(git_repo, &branch, &context, &detail.request)
        .map_err(|error| recover("save_local_metadata", error))?;
    let detail = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )
    .map_err(|error| recover("refresh_request", error))?;
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(request_detail_lines_for_response(&detail));
    let result = DetailResult {
        repo: context.repo,
        request: detail.request,
        activity: None,
    };
    Ok(RequestCommandOutcome::new(
        "request.push",
        RequestCommandResult::Detail(result),
        human_lines,
    ))
}

pub(super) fn ensure_public_request_paths_allowed(
    git_repo: &GitRepo,
    detail: &crate::api::RequestDetailResponse,
    current_main_oid: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    if detail.request.audience != RequestAudience::Public {
        return Ok(());
    }
    let changed_paths = request_side_changed_file_paths(
        git_repo,
        detail.request.base_main_oid.as_str(),
        current_main_oid,
        request_head_oid,
    )?;
    let protected_paths = changed_paths
        .into_iter()
        .filter_map(|path| {
            let scope_path = ScopePath::parse(format!("/{path}")).ok()?;
            is_public_request_protected_path(&scope_path).then_some(path)
        })
        .collect::<Vec<_>>();
    if protected_paths.is_empty() {
        return Ok(());
    }

    let message = format!(
        "public request cannot change maintainer-controlled paths: {}",
        protected_paths.join(", ")
    );
    let response = ErrorResponse::new(ErrorCode::ProtectedPath, message)
        .with_paths(protected_paths)
        .with_instruction(
            "Move maintainer-controlled changes to a maintainer-authored change, then retry.",
        );
    Err(crate::error::CliError::new(response).into())
}
