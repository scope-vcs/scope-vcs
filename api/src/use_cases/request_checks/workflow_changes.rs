use crate::{
    error::ApiError,
    git::{
        command::{run_git_output, successful_git_output},
        request_refs::with_request_revision_store_repo,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepoRecord,
    requests::{
        GitHubCheckTarget, Request, RequestCheckEvaluation, RequestComparison, RequestRevision,
        changes_github_workflows,
    },
    views::Views,
};
use std::path::Path;

pub(super) async fn changes_github_workflow_files(
    state: &AppState,
    repo: &RepoRecord,
    request: &Request,
    evaluation: &RequestCheckEvaluation,
    views: &Views,
    staging_repo: Option<&Path>,
) -> Result<bool, ApiError> {
    const ACTION: &str = "reading request workflow changes";
    let Some(revision) = state
        .metadata
        .requests()
        .request_revision_with_head(&request.id, &request.head_oid)
        .await?
    else {
        return Ok(true);
    };
    let execution_base_oid = match GitHubCheckTarget::for_request(request, views) {
        GitHubCheckTarget::Head => None,
        GitHubCheckTarget::CheckCommit => {
            let Some(base) = &evaluation.check_commit_base else {
                return Ok(true);
            };
            Some(base.view_base_oid.clone())
        }
    };
    let request_view = request.view.clone();
    let inspect = move |path: &Path, revision: &RequestRevision| {
        let comparison = RequestComparison::request(revision, request_view);
        let range = comparison.membership();
        let output = successful_git_output(
            run_git_output(
                Some(path),
                &[
                    "diff",
                    "--name-only",
                    "-z",
                    execution_base_oid
                        .as_deref()
                        .or(comparison.identity().base_oid.as_deref())
                        .ok_or_else(|| {
                            ApiError::conflict("request comparison has no review base")
                        })?,
                    range.head_oid(),
                    "--",
                    ".github/workflows",
                ],
                ACTION,
            )?,
            ACTION,
        )?;
        Ok(changes_github_workflows(
            output
                .stdout
                .split(|byte| *byte == 0)
                .filter_map(|path| std::str::from_utf8(path).ok()),
        ))
    };
    if let Some(path) = staging_repo {
        let path = path.to_path_buf();
        crate::git::blocking::run(move || inspect(&path, &revision)).await
    } else {
        with_request_revision_store_repo(state, &repo.incarnation(), request, &revision, inspect)
            .await
    }
}
