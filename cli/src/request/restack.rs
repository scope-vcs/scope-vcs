use super::*;
use crate::git_repo::{
    active_rebase_branch, git_worktree_directory, rebase_layer, require_rebase_ready, resume_rebase,
};
use args::RequestRestackArgs;
use scope_api_contract::RequestBase;

pub(in crate::request) mod state;
use state::PendingRestack;

pub(super) fn run(
    repo: &GitRepo,
    api: ApiSession<'_>,
    args: RequestRestackArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let pending = state::load(repo)?;
    if args.continue_rebase || args.abort || args.discard_selection {
        let pending = pending.ok_or_else(|| {
            crate::error::CliError::usage("there is no pending restack selection")
        })?;
        if args
            .remote
            .as_ref()
            .is_some_and(|remote| remote != &pending.remote)
        {
            return Err(crate::error::CliError::usage(
                "restack recovery belongs to a different remote",
            )
            .into());
        }
        return if args.discard_selection {
            discard_selection(repo, pending)
        } else {
            resume(repo, pending, args.abort)
        };
    }
    if let Some(pending) = pending {
        return Err(recovery(
            repo,
            &pending,
            "inspect_pending_restack",
            "a previous restack needs inspection or publication",
        ));
    }
    ensure_clean_working_tree(repo, "scope request restack")?;
    let branch = current_branch(repo).map_err(|_| {
        crate::error::CliError::usage("restack requires an attached request branch")
    })?;
    let context = load_context(Some(repo), api, args.remote.as_deref())?;
    local::require_git_remote(&context)?;
    let id = local::attached_request_id(&branch, &context)?;
    let request = get_request(api, &context.target.owner, &context.target.repo, &id)?.request;
    if !request.permissions.can_push_branch {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            "you cannot restack this request",
        ))
        .into());
    }
    let selector = args.onto.unwrap_or_else(|| match &request.base {
        RequestBase::Main { .. } => "main".to_string(),
        RequestBase::Request { request_id, .. } => request_id.clone(),
    });
    let head = head_oid(repo)?;
    stack::require_contains_pin(repo, &request.base, &head)?;
    require_rebase_ready(repo, &branch, stack::base_head(&request.base), &head)?;
    let selected = stack::select_base(repo, api, &context, &request, &selector, None)?;
    require_preserved_main(repo, &request, &selected.base)?;
    let history =
        crate::api::request_revisions(api, api_target(&context, &request.id), None, None, None)?;
    let revision = history.review_revision_id;
    let backup_ref = format!(
        "refs/scope/restacks/{}/{}",
        request.id,
        new_client_mutation_id("restack")?
    );
    let pending = PendingRestack {
        owner: context.target.owner,
        repository: context.target.repo,
        remote: context.target.remote,
        request_id: request.id,
        branch,
        worktree: git_worktree_directory(repo)?,
        original_head: head,
        original_base: request.base,
        expected_revision: revision,
        expected_head: request.head_oid.to_string(),
        expected_base_version: request.base_version,
        selected,
        backup_ref,
    };
    run_git_in_repo(
        repo,
        &[
            "update-ref",
            &pending.backup_ref,
            &pending.original_head,
            &"0".repeat(40),
        ],
    )?;
    state::save(repo, &pending)
        .map_err(|error| recovery(repo, &pending, "save_selection", &error.to_string()))?;
    let output = rebase_layer(
        repo,
        stack::base_head(&pending.selected.base),
        stack::base_head(&pending.original_base),
        &pending.branch,
    )
    .map_err(|error| recovery(repo, &pending, "rebase", &error.to_string()))?;
    if !output.status.success() {
        return Err(recovery(
            repo,
            &pending,
            "rebase",
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    finish(repo, pending, false)
}

pub(super) fn require_preserved_main(
    repo: &GitRepo,
    request: &crate::api::RequestSummaryResponse,
    base: &RequestBase,
) -> anyhow::Result<()> {
    if !try_run_git_in_repo(
        repo,
        &[
            "merge-base",
            "--is-ancestor",
            request.main_ancestor_oid.as_str(),
            stack::base_head(base),
        ],
    )? {
        return Err(crate::error::CliError::usage("the selected base predates this request's required main ancestry; select main or update the parent first").into());
    }
    Ok(())
}

fn resume(
    repo: &GitRepo,
    pending: PendingRestack,
    abort: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let active = active_rebase_branch(repo)?;
    if active.is_none() {
        if abort {
            if head_oid(repo)? == pending.original_head {
                return finish(repo, pending, true);
            }
            return Err(recovery(
                repo,
                &pending,
                "inspect_completed_restack",
                "Git has no active rebase; inspect the backup and current HEAD before recovering manually",
            ));
        }
        return finish(repo, pending, false);
    }
    if active.as_deref() != Some(pending.branch.as_str())
        || crate::git_repo::rebase_original_head(repo)?.as_deref()
            != Some(pending.original_head.as_str())
    {
        return Err(recovery(
            repo,
            &pending,
            "verify_rebase",
            "the active Git rebase belongs to another branch",
        ));
    }
    let output = resume_rebase(repo, abort)
        .map_err(|error| recovery(repo, &pending, "resume_rebase", &error.to_string()))?;
    if !output.status.success() {
        return Err(recovery(
            repo,
            &pending,
            "resume_rebase",
            &String::from_utf8_lossy(&output.stderr),
        ));
    }
    finish(repo, pending, abort)
}

fn discard_selection(
    repo: &GitRepo,
    pending: PendingRestack,
) -> anyhow::Result<RequestCommandOutcome> {
    crate::git_repo::require_no_git_operation(repo)
        .map_err(|error| recovery(repo, &pending, "verify_git_operation", &error.to_string()))?;
    if current_branch(repo)? != pending.branch {
        return Err(recovery(
            repo,
            &pending,
            "verify_branch",
            "return to the original restack branch before discarding its selection",
        ));
    }
    let head = head_oid(repo)?;
    state::clear(repo)
        .map_err(|error| recovery(repo, &pending, "clear_selection", &error.to_string()))?;
    Ok(restack_outcome(pending, head, "selection_discarded"))
}

fn finish(
    repo: &GitRepo,
    pending: PendingRestack,
    aborted: bool,
) -> anyhow::Result<RequestCommandOutcome> {
    let head = head_oid(repo)?;
    if current_branch(repo)? != pending.branch {
        return Err(recovery(
            repo,
            &pending,
            "verify_branch",
            "Git did not restore the attached request branch",
        ));
    }
    if aborted {
        if head != pending.original_head {
            return Err(recovery(
                repo,
                &pending,
                "verify_abort",
                "the aborted rebase did not restore the original head",
            ));
        }
        state::clear(repo)
            .map_err(|error| recovery(repo, &pending, "clear_selection", &error.to_string()))?;
    } else {
        stack::require_contains_pin(repo, &pending.selected.base, &head)?;
    }
    let status = if aborted {
        "aborted"
    } else {
        "ready_to_publish"
    };
    Ok(restack_outcome(pending, head, status))
}

fn restack_outcome(
    pending: PendingRestack,
    head: String,
    status: &'static str,
) -> RequestCommandOutcome {
    let lines = vec![
        format!("Request {} restack: {status}", pending.request_id),
        format!(
            "Backup: {} ({})",
            pending.backup_ref,
            short_oid(&pending.original_head)
        ),
        match status {
            "aborted" => "The original head was restored.".to_string(),
            "selection_discarded" => format!(
                "Preserved HEAD {head}. Inspect the new base, run git rebase --onto <new-base> {}, then scope request push --base <request|main>.",
                stack::base_head(&pending.selected.base)
            ),
            _ => format!(
                "Inspect the replay, then run scope request push --remote {} --request {}",
                pending.remote, pending.request_id
            ),
        },
    ];
    let result = RestackResult {
        request_id: pending.request_id,
        branch: pending.branch,
        status,
        head_oid: head,
        original_head_oid: pending.original_head,
        original_base: pending.original_base,
        selected_base: pending.selected.base,
        backup_ref: pending.backup_ref,
        remote: pending.remote,
    };
    RequestCommandOutcome::new(
        "request.restack",
        RequestCommandResult::Restack(result),
        lines,
    )
}

pub(super) fn recovery(
    repo: &GitRepo,
    pending: &PendingRestack,
    step: &str,
    message: &str,
) -> anyhow::Error {
    crate::error::CliError::partial(message, serde_json::json!({
        "request_id": pending.request_id,
        "branch": pending.branch,
        "failed_step": step,
        "original_head_oid": pending.original_head,
        "original_base": pending.original_base,
        "selected_base": pending.selected.base,
        "backup_ref": pending.backup_ref,
        "head_oid": head_oid(repo).ok(),
        "continue_command": ["scope", "request", "restack", "--continue"],
        "abort_command": ["scope", "request", "restack", "--abort"],
        "discard_selection_command": ["scope", "request", "restack", "--discard-selection"],
        "publish_command": ["scope", "request", "push", "--remote", pending.remote, "--request", pending.request_id],
    })).into()
}

pub(super) fn publication_failure(
    repo: &GitRepo,
    pending: &PendingRestack,
    cause: anyhow::Error,
) -> anyhow::Error {
    let recovery = crate::error::json_response(&recovery(
        repo,
        pending,
        "publish_restack",
        &cause.to_string(),
    ))
    .recovery
    .expect("restack recovery receipt");
    crate::error::CliError::with_recovery(crate::error::response(&cause), recovery).into()
}
