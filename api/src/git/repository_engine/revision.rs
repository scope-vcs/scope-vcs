use super::RepositoryEngine;
use crate::{
    error::ApiError,
    git::{GitContext, cache::GitRepoHandle, command::run_git},
};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitHead, GitPackSpan},
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{fs, path::Path, sync::Arc};

/// Pins the requested commit and keeps its shared object database alive.
/// Callers must resolve authorization before asking the engine for a revision.
pub(crate) struct GitRevision {
    repository: GitRepoHandle,
    head_oid: String,
}

impl RepositoryEngine {
    pub(crate) async fn materialize_revision<C: GitContext>(
        self: &Arc<Self>,
        context: &C,
        incarnation: &RepositoryIncarnation,
        head: &GitHead,
        pack_spans: &[GitPackSpan],
    ) -> Result<GitRevision, ApiError> {
        Ok(GitRevision {
            repository: self
                .materialize_repository(context, incarnation, head, pack_spans)
                .await?,
            head_oid: head.head_oid.clone(),
        })
    }
}

impl GitRevision {
    /// Creates a private, temporary ref view. Keep this handle alive until Git
    /// exits. Public projections must still copy only their authorized objects.
    pub(crate) fn create_view(&self, path: &Path) -> Result<(), ApiError> {
        run_git(
            None,
            &[
                "init",
                "--bare",
                path.to_str()
                    .ok_or_else(|| ApiError::internal_message("Git revision path is not UTF-8"))?,
            ],
            "initializing pinned Git revision",
        )?;
        let objects =
            fs::canonicalize(self.repository.join("objects")).map_err(ApiError::internal)?;
        fs::write(
            path.join("objects/info/alternates"),
            format!("{}\n", objects.display()),
        )
        .map_err(ApiError::internal)?;
        run_git(
            Some(path),
            &[
                "update-ref",
                &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                &self.head_oid,
            ],
            "pinning requested Git revision",
        )
    }
}
