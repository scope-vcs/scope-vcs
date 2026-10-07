use crate::{
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        check_commit::write_check_commit,
        command::{git_stdout_text, run_git},
        repository_git::RepositoryGit,
        request_ref_view_safety::{RequestView, view_contribution_base},
        request_refs::with_request_revision_store_repo,
        storage::{receive_pack_staging_repo_path, remove_dir_if_exists},
    },
    persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{
        CheckCommitBase, GitHubTestedCommit, Request, RequestRevision, canonical_request_ref,
    },
};
use std::{
    fs,
    path::{Path, PathBuf},
};

const CHECK_HEAD_REF: &str = "refs/scope/internal/check-head";

pub(crate) async fn view_tested_commit(
    state: &AppState,
    git: &RepositoryGit,
    request: &Request,
    revision: &RequestRevision,
) -> Result<GitHubTestedCommit, ApiError> {
    let staging = CheckStaging::open(state, &git.incarnation, request, revision).await?;
    let built = async {
        let views = git.views(state).await?;
        let view_base_oid = view_contribution_base(
            RequestView::new(git, &views, &request.view),
            state,
            &staging.path,
            &revision.new_head_oid,
        )
        .await?;
        let base = CheckCommitBase::new(staging.canonical_main_oid.clone(), view_base_oid)?;
        let path = staging.path.clone();
        let request_id = request.id.clone();
        let head_oid = revision.new_head_oid.clone();
        let written = {
            let base = base.clone();
            crate::git::blocking::run(move || {
                write_check_commit(&path, &request_id, &head_oid, &base)
            })
            .await?
        };
        Ok(match written {
            Some(oid) => GitHubTestedCommit::CheckCommit { oid, base },
            None => GitHubTestedCommit::Conflict,
        })
    }
    .await;
    staging.remove().await;
    built
}

pub(crate) async fn with_check_commit<T: Send + 'static>(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
    base: &CheckCommitBase,
    expected_oid: &str,
    action: impl FnOnce(&Path) -> T + Send + 'static,
) -> Result<T, ApiError> {
    let staging = CheckStaging::open(state, incarnation, request, revision).await?;
    let path = staging.path.clone();
    let request_id = request.id.clone();
    let head_oid = revision.new_head_oid.clone();
    let base = base.clone();
    let expected_oid = expected_oid.to_string();
    let result = crate::git::blocking::run(move || {
        if write_check_commit(&path, &request_id, &head_oid, &base)?.as_deref()
            != Some(expected_oid.as_str())
        {
            return Err(ApiError::internal_message(
                "the check commit built again differs from the tested commit",
            ));
        }
        Ok(action(&path))
    })
    .await;
    staging.remove().await;
    result
}

struct CheckStaging {
    path: PathBuf,
    canonical_main_oid: String,
    canonical: GitRepoHandle,
}

impl CheckStaging {
    async fn open(
        state: &AppState,
        incarnation: &RepositoryIncarnation,
        request: &Request,
        revision: &RequestRevision,
    ) -> Result<Self, ApiError> {
        let (Some(head), spans) = state
            .metadata
            .repositories()
            .repository_content_source(incarnation)
            .await?
        else {
            return Err(ApiError::conflict("repo has no accepted Git head"));
        };
        let canonical = state
            .repository_engine
            .materialize_repository(state, incarnation, &head, &spans)
            .await?;
        let staging = Self {
            path: receive_pack_staging_repo_path(state, incarnation)?,
            canonical_main_oid: head.head_oid,
            canonical,
        };
        let initialized = {
            let path = staging.path.clone();
            let canonical_objects = staging.canonical.join("objects");
            crate::git::blocking::run(move || {
                if let Some(parent) = path.parent() {
                    ensure_private_dir(parent)?;
                }
                run_git(
                    None,
                    &["init", "--quiet", "--bare", path.to_string_lossy().as_ref()],
                    "initializing check commit repository",
                )?;
                let canonical_objects =
                    fs::canonicalize(canonical_objects).map_err(ApiError::internal)?;
                fs::write(
                    path.join("objects/info/alternates"),
                    format!("{}\n", canonical_objects.display()),
                )
                .map_err(ApiError::internal)
            })
            .await
        };
        let fetched = match initialized {
            Ok(()) => {
                let path = staging.path.clone();
                let request_ref = canonical_request_ref(&request.name);
                with_request_revision_store_repo(
                    state,
                    incarnation,
                    request,
                    revision,
                    move |revision_repo, revision| {
                        fetch_head(&path, revision_repo, &request_ref, &revision.new_head_oid)
                    },
                )
                .await
            }
            Err(error) => Err(error),
        };
        match fetched {
            Ok(()) => Ok(staging),
            Err(error) => {
                staging.remove().await;
                Err(error)
            }
        }
    }

    async fn remove(self) {
        let path = self.path.clone();
        if let Err(error) = crate::git::blocking::run(move || remove_dir_if_exists(&path)).await {
            tracing::warn!(
                path = %self.path.display(),
                error = %error.operator_diagnostic(),
                "could not remove a check commit repository"
            );
        }
    }
}

fn fetch_head(
    staging: &Path,
    revision_repo: &Path,
    request_ref: &str,
    head_oid: &str,
) -> Result<(), ApiError> {
    run_git(
        Some(staging),
        &[
            "fetch",
            "--quiet",
            "--no-tags",
            revision_repo.to_string_lossy().as_ref(),
            &format!("+{request_ref}:{CHECK_HEAD_REF}"),
        ],
        "fetching the contribution for its check commit",
    )?;
    let fetched = git_stdout_text(
        staging,
        &["rev-parse", "--verify", CHECK_HEAD_REF],
        "reading the contribution for its check commit",
    )?;
    if fetched.trim() != head_oid {
        return Err(ApiError::infrastructure_unavailable(
            "request revision does not hold its head",
        ));
    }
    Ok(())
}
