use super::*;
use crate::api::ApiSession;

pub(super) fn start_request_branch(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    args: RequestStartArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let context = load_context(Some(git_repo), api, args.remote.as_deref())?;
    local::require_git_remote(&context)?;
    let (view, base, base_oid) = if let Some(selector) = args.base {
        let (parent, pin) =
            stack::current_parent(git_repo, api, &context, selector, args.view.as_ref(), None)?;
        let oid = parent.head_oid.as_str().to_string();
        (parent.view, Some(pin.base), oid)
    } else {
        let view = start_view(&context.repo.access, &context.views, args.view)?;
        let oid = refresh_main_projection(git_repo, &context.target, &view, api.token)?;
        (view, None, oid)
    };
    let name = args.name.trim().to_string();
    scope_domain::requests::validate_chosen_request_name(&name)
        .map_err(|error| crate::error::CliError::usage(error.message))?;
    let branch = if args.current_branch {
        local::adoptable_current_branch(git_repo, &base_oid)?
    } else {
        let local_ref = format!("refs/heads/{name}");
        if try_run_git_in_repo(git_repo, &["show-ref", "--verify", "--quiet", &local_ref])? {
            bail!("local branch '{name}' already exists");
        }
        name.clone()
    };
    let response = api_start_request(
        api,
        StartRequestParams {
            base,
            owner: &context.target.owner,
            repo: &context.target.repo,
            name,
            title: args.title,
            view: view.clone(),
        },
    )?;
    if !args.current_branch
        && let Err(switch_error) = run_git_in_repo(
            git_repo,
            &["switch", "--quiet", "--no-track", "-c", &branch, &base_oid],
        )
    {
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
        &view,
        api.token,
        &request_head_oid,
        response.request.head_oid.as_str(),
        &response.request.id,
        &response.request.name,
    )
    .map_err(|error| recover("push_request_head", error))?;
    if args.current_branch {
        update_request_remote_ref(
            git_repo,
            &context.target,
            &response.request.name,
            &request_head_oid,
        )
        .map_err(|error| recover("update_remote_ref", error))?;
    } else {
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
    }
    let detail = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &response.request.id,
    )
    .map_err(|error| {
        recovery::request_partial(
            &context,
            &response.request,
            &branch,
            "refresh_request",
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
            view_label(&context.views, &view),
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
        request: detail.request,
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
    base_revision: Option<String>,
) -> anyhow::Result<RequestCommandOutcome> {
    if !crate::execution::json() {
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
    let current_main_oid =
        refresh_main_projection(git_repo, &context.target, &detail.request.view, api.token)?;
    ensure_request_paths_allowed(
        git_repo,
        &context.views,
        &detail.request,
        &current_main_oid,
        &request_head_oid,
    )?;
    let already_accepted = if let Some(revision) = base_revision.as_deref() {
        stack::retry_pending_context(
            git_repo,
            api,
            &context,
            &detail.request,
            &request_head_oid,
            revision,
        )? || stack::accepted_pin(
            api,
            &context,
            &detail.request,
            &request_head_oid,
            revision,
            None,
        )?
    } else {
        false
    };
    let pin = if already_accepted {
        None
    } else if let Some(revision) = base_revision.as_deref() {
        let scope_api_contract::RequestBase::Request { request_id, .. } = &detail.request.base
        else {
            return Err(crate::error::CliError::usage(
                "--base-revision requires a request with a parent",
            )
            .into());
        };
        let (_, pin) = stack::current_parent(
            git_repo,
            api,
            &context,
            request_id.clone(),
            Some(&detail.request.view),
            Some(revision),
        )?;
        stack::require_contains_pin(git_repo, &pin.base, &request_head_oid)?;
        Some(pin)
    } else {
        stack::require_contains_pin(git_repo, &detail.request.base, &request_head_oid)?;
        None
    };
    let expected_head_oid = last_seen_request_head(
        git_repo,
        &context.target,
        &detail.request,
        &branch,
        &request_head_oid,
    )?;
    if !already_accepted && let Some(pin) = &pin {
        if request_head_oid == detail.request.head_oid.as_str() {
            stack::publish_context(
                git_repo,
                api,
                &context,
                &detail.request,
                pin,
                &request_head_oid,
            )?;
        } else {
            let result = crate::git_repo::push_head_to_ref_with_headers(
                &context
                    .target
                    .url_for_view(&detail.request.view.clone().into()),
                &request_head_oid,
                &format!("refs/heads/{}", detail.request.name),
                &expected_head_oid,
                api.token,
                &stack::publication_headers(&detail.request, pin),
            )
            .map_err(|error| local::request_push_error(error, &detail.request.id));
            let revision = base_revision.as_deref().expect("explicit pin");
            let expected_base_version = if detail.request.base == pin.base {
                detail.request.base_version
            } else {
                detail
                    .request
                    .base_version
                    .checked_add(1)
                    .context("request base version cannot advance")?
            };
            if !stack::accepted_pin(
                api,
                &context,
                &detail.request,
                &request_head_oid,
                revision,
                Some((&pin.base, expected_base_version)),
            )? {
                result?;
                return Err(crate::error::CliError::new(ErrorResponse::new(
                    ErrorCode::Conflict,
                    "revision history cannot confirm the intended head and parent pin",
                ))
                .into());
            }
        }
    } else if !already_accepted {
        push_request_head(
            &context.target,
            &detail.request.view,
            api.token,
            &request_head_oid,
            &expected_head_oid,
            &detail.request.id,
            &detail.request.name,
        )?;
    }
    let recover = |stage, error| {
        recovery::request_partial(&context, &detail.request, &branch, stage, true, error)
    };
    update_request_remote_ref(
        git_repo,
        &context.target,
        &detail.request.name,
        &request_head_oid,
    )
    .map_err(|error| recover("update_remote_ref", error))?;
    let detail = get_request(
        api,
        &context.target.owner,
        &context.target.repo,
        &request_id,
    )
    .map_err(|error| recover("refresh_request", error))?;
    let mut human_lines = repo_access_lines(&context.repo);
    human_lines.extend(request_detail_lines(&detail.request, &context.views));
    let result = DetailResult {
        repo: context.repo,
        request: detail.request,
        activity: None,
        auto_merge: None,
    };
    Ok(RequestCommandOutcome::new(
        "request.push",
        RequestCommandResult::Detail(result),
        human_lines,
    ))
}

pub(super) fn ensure_request_paths_allowed(
    git_repo: &GitRepo,
    views: &Views,
    request: &crate::api::RequestSummaryResponse,
    current_main_oid: &str,
    request_head_oid: &str,
) -> anyhow::Result<()> {
    let view = scope_domain::views::ViewId::from(request.view.clone());
    if &view == views.full() {
        return Ok(());
    }
    let changed_paths = request_side_changed_file_paths(
        git_repo,
        request.main_ancestor_oid.as_str(),
        current_main_oid,
        request_head_oid,
    )?;
    let protected_paths = changed_paths
        .into_iter()
        .filter_map(|path| {
            let scope_path = ScopePath::parse(format!("/{path}")).ok()?;
            is_request_protected_path(&scope_path).then_some(path)
        })
        .collect::<Vec<_>>();
    if protected_paths.is_empty() {
        return Ok(());
    }

    let message = format!(
        "{} requests cannot change maintainer-controlled paths: {}",
        views.display_name(&view),
        protected_paths.join(", ")
    );
    let response = ErrorResponse::new(ErrorCode::ProtectedPath, message)
        .with_paths(protected_paths)
        .with_instruction(
            "Move maintainer-controlled changes to a maintainer-authored change, then retry.",
        );
    Err(crate::error::CliError::new(response).into())
}
