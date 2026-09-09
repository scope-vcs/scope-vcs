use super::*;
use crate::api::ApiSession;
use crate::{
    api::{RequestFileDiffParams, request_file_diff, request_revisions},
    git_repo::{
        branch_config_value, fetch_scope_remote_with_bearer, run_git_in_repo, scope_remote_head_oid,
    },
};
use args::{RequestCheckoutArgs, RequestDiffArgs};
use scope_api_contract::RequestRevisionInspectionState;
use text::terminal_text;

pub(super) fn checkout_request(
    git_repo: &GitRepo,
    api: ApiSession<'_>,
    args: RequestCheckoutArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    ensure_clean_working_tree(git_repo, "scope request checkout")?;
    let (context, request_id, detail) = load_exact_request(Some(git_repo), api, args.target)?;
    if context.target.remote.is_empty() {
        return Err(crate::error::CliError::usage(
            "request checkout requires a configured Scope Git remote",
        )
        .into());
    }
    if !detail.request.permissions.can_pull_branch {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::Forbidden,
            "you do not have permission to check out this request branch",
        ))
        .into());
    }
    let branch = args.branch.unwrap_or_else(|| detail.request.name.clone());
    if !try_run_git_in_repo(git_repo, &["check-ref-format", "--branch", &branch])? {
        return Err(crate::error::CliError::usage("invalid local branch name").into());
    }
    let local_ref = format!("refs/heads/{branch}");
    let exists = try_run_git_in_repo(git_repo, &["show-ref", "--verify", "--quiet", &local_ref])?;
    if exists
        && branch_config_value(git_repo, &branch, "scopeRequestId")?.as_deref() != Some(&request_id)
    {
        return Err(crate::error::CliError::usage(format!(
            "local branch '{branch}' is not linked to request {request_id}; choose a new name with --branch"
        )).into());
    }
    fetch_scope_remote_with_bearer(
        git_repo,
        &context.target.permissioned_url,
        &context.target.remote,
        &detail.request.name,
        api.token,
    )?;
    let fetched = scope_remote_head_oid(git_repo, &context.target.remote, &detail.request.name)?
        .context("request fetch did not produce a remote ref")?;
    if fetched != detail.request.head_oid.as_str() {
        bail!(
            "request {request_id} changed during checkout; no branch was switched; retry scope request checkout --request {request_id}"
        );
    }
    let recovery = |step: &str, error: anyhow::Error| -> anyhow::Error {
        crate::error::CliError::partial(
            format!("request {request_id} was fetched, but {step} failed: {error}. After resolving the Git error, retry `scope request checkout --remote {} --request {request_id} --branch {branch}`", context.target.remote),
            serde_json::json!({
                "request_id": request_id,
                "branch": branch,
                "failed_step": step,
                "fetched_head_oid": fetched,
                "retry_command": ["scope", "request", "checkout", "--remote", context.target.remote, "--request", request_id, "--branch", branch],
            }),
        ).into()
    };
    // Record identity before creating a branch so any later failure can be retried
    // without mistaking the partially configured branch for an unrelated branch.
    store_request_metadata(git_repo, &branch, &context, &detail.request)
        .map_err(|error| recovery("save_local_metadata", error))?;
    switch_request_branch(git_repo, &branch, &fetched, exists)
        .map_err(|error| recovery("switch_local_branch", error))?;
    track_request_branch_ref(
        git_repo,
        &branch,
        &context.target,
        &detail.request.name,
        &fetched,
    )
    .map_err(|error| recovery("configure_tracking", error))?;
    Ok(RequestCommandOutcome::new(
        "request.checkout",
        RequestCommandResult::Checkout(CheckoutResult {
            repo: context.repo,
            request: detail.request,
            branch: branch.clone(),
            head_oid: fetched.clone(),
        }),
        vec![format!(
            "Checked out request {request_id} on {branch} ({})",
            short_oid(&fetched)
        )],
    ))
}

fn switch_request_branch(
    git_repo: &GitRepo,
    branch: &str,
    head: &str,
    exists: bool,
) -> anyhow::Result<()> {
    if exists {
        let local_ref = format!("refs/heads/{branch}");
        if !try_run_git_in_repo(git_repo, &["merge-base", "--is-ancestor", &local_ref, head])? {
            return Err(crate::error::CliError::usage(format!("local branch '{branch}' has unpublished or diverged commits; choose a new --branch to preserve it")).into());
        }
        run_git_in_repo(git_repo, &["switch", "--quiet", branch])?;
        run_git_in_repo(git_repo, &["merge", "--quiet", "--ff-only", head])?;
    } else {
        run_git_in_repo(
            git_repo,
            &["switch", "--quiet", "--no-track", "-c", branch, head],
        )?;
    }
    Ok(())
}

pub(super) fn diff_request(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    args: RequestDiffArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, _) = load_exact_request(git_repo, api, args.target)?;
    let target = RequestTarget {
        owner: &context.target.owner,
        repo: &context.target.repo,
        request_id: &request_id,
    };
    let revisions = request_revisions(
        api,
        target,
        args.revision.as_deref(),
        args.commit.as_deref(),
    )?;
    let selected = revisions.review_revision_id.as_deref().and_then(|id| {
        revisions
            .revisions
            .iter()
            .find(|revision| revision.id == id)
    });
    if args.path.is_none()
        && let Some(commit) = args.commit.as_deref()
        && !selected.is_some_and(|revision| revision.commits.iter().any(|item| item.oid == commit))
    {
        return Err(crate::error::CliError::new(ErrorResponse::new(
            ErrorCode::NotFound,
            "the selected commit is not available in the server's visible revision inspection",
        ))
        .into());
    }
    let mut lines = Vec::new();
    let mut files = Vec::new();
    if let Some(revision) = selected {
        lines.push(format!(
            "Request {request_id}, revision {}",
            terminal_text(&revision.id)
        ));
        if revision.inspection != RequestRevisionInspectionState::Complete {
            lines.push(format!(
                "Inspection is {:?}; the server has not provided a complete file list.",
                revision.inspection
            ));
        }
        for commit in revision
            .commits
            .iter()
            .filter(|commit| args.commit.as_deref().is_none_or(|oid| commit.oid == oid))
        {
            lines.push(format!(
                "{} {}",
                short_oid(&commit.oid),
                terminal_text(&commit.message)
            ));
            if args.path.is_none() {
                for changed in &commit.files {
                    let diff = request_file_diff(
                        api,
                        RequestFileDiffParams {
                            target,
                            revision: &revision.id,
                            commit: &commit.oid,
                            path: &changed.path,
                        },
                    )?;
                    lines.extend(super::diff::file_diff_lines(&diff));
                    files.push(InspectedFile {
                        commit_oid: commit.oid.clone(),
                        diff,
                    });
                }
            }
            if commit.files_truncated {
                lines.push("  File list is truncated.".to_string());
            }
        }
        if let (Some(commit), Some(path)) = (args.commit.as_deref(), args.path.as_deref()) {
            let diff = request_file_diff(
                api,
                RequestFileDiffParams {
                    target,
                    revision: &revision.id,
                    commit,
                    path,
                },
            )?;
            lines.extend(super::diff::file_diff_lines(&diff));
            files.push(InspectedFile {
                commit_oid: commit.to_string(),
                diff,
            });
        }
    } else {
        lines.push("No visible request revisions.".to_string());
    }
    Ok(RequestCommandOutcome::new(
        "request.diff",
        RequestCommandResult::Diff(DiffResult {
            repo: context.repo,
            request_id,
            revisions,
            files,
        }),
        lines,
    ))
}

pub(super) fn request_checks(
    git_repo: Option<&GitRepo>,
    api: ApiSession<'_>,
    target: RequestTargetArgs,
) -> anyhow::Result<RequestCommandOutcome> {
    let (context, request_id, detail) = load_exact_request(git_repo, api, target)?;
    let workflow_runs_available = matches!(
        context.repo.access.actor,
        crate::api::RepositoryActor::Owner | crate::api::RepositoryActor::Member
    );
    let mut runs = Vec::new();
    if workflow_runs_available {
        let mut cursor = None;
        loop {
            let page = crate::api::run_history(
                api,
                &context.target.owner,
                &context.target.repo,
                None,
                100,
                cursor.as_deref(),
            )?;
            runs.extend(
                page.runs
                    .into_iter()
                    .filter(|run| run.git_oid == detail.request.head_oid.as_str()),
            );
            let Some(next) = page.next_cursor else {
                break;
            };
            cursor = Some(next);
        }
    }
    let mut lines = vec![
        format!(
            "Request {request_id}, head {}",
            short_oid(detail.request.head_oid.as_str())
        ),
        format!("Mergeability: {:?}", detail.request.mergeability.status),
    ];
    if let Some(reason) = &detail.request.mergeability.reason {
        lines.push(terminal_text(reason));
    }
    for run in &runs {
        lines.push(format!(
            "{}: {:?} ({})",
            terminal_text(&run.workflow_name),
            run.state,
            terminal_text(&run.id)
        ));
    }
    if !workflow_runs_available {
        lines.push("Workflow runs are visible to repository maintainers.".to_string());
    } else if runs.is_empty() {
        lines.push("No workflow runs for this request head. Request pushes do not automatically start workflows.".to_string());
    }
    Ok(RequestCommandOutcome::new(
        "request.checks",
        RequestCommandResult::Checks(ChecksResult {
            repo: context.repo,
            request_id,
            head_oid: detail.request.head_oid.to_string(),
            mergeability: detail.request.mergeability,
            workflow_runs_available,
            runs,
        }),
        lines,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;
    use std::fs;

    #[test]
    fn checkout_preserves_unpublished_branch_commits() {
        let dir = TestDir::git_repo("request-checkout-preserve", "main");
        fs::write(dir.path().join("file.txt"), "initial\n").unwrap();
        dir.run_git(["add", "."]);
        dir.run_git([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "initial",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };
        let remote_head = head_oid(&repo).unwrap();
        dir.run_git(["switch", "-c", "request-change"]);
        fs::write(dir.path().join("file.txt"), "unpublished\n").unwrap();
        dir.run_git(["add", "."]);
        dir.run_git([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "unpublished",
        ]);
        let unpublished = head_oid(&repo).unwrap();
        dir.run_git(["switch", "main"]);
        let error = switch_request_branch(&repo, "request-change", &remote_head, true).unwrap_err();
        assert!(error.to_string().contains("unpublished or diverged"));
        assert_eq!(current_branch(&repo).unwrap(), "main");
        dir.run_git(["switch", "request-change"]);
        assert_eq!(head_oid(&repo).unwrap(), unpublished);
    }

    #[test]
    fn checkout_creates_and_fast_forwards_without_losing_content() {
        let dir = TestDir::git_repo("request-checkout-ff", "main");
        fs::write(dir.path().join("file.txt"), "initial\n").unwrap();
        dir.run_git(["add", "."]);
        dir.run_git([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "initial",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };
        let base = head_oid(&repo).unwrap();
        switch_request_branch(&repo, "request-change", &base, false).unwrap();
        dir.run_git(["switch", "main"]);
        fs::write(dir.path().join("file.txt"), "remote advance\n").unwrap();
        dir.run_git(["add", "."]);
        dir.run_git([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.test",
            "commit",
            "-m",
            "advance",
        ]);
        let next = head_oid(&repo).unwrap();
        switch_request_branch(&repo, "request-change", &next, true).unwrap();
        assert_eq!(current_branch(&repo).unwrap(), "request-change");
        assert_eq!(head_oid(&repo).unwrap(), next);
        assert_eq!(
            fs::read_to_string(dir.path().join("file.txt")).unwrap(),
            "remote advance\n"
        );
    }
}
