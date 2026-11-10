use super::{PoolInstall, PoolRepos, install_missing_request_packs, request_object_repos};
use crate::{
    error::ApiError,
    git::{
        blocking, cache::GitRepoHandle, command::run_git, pool_install::present_commits,
        repository_git::RepositoryGit,
    },
    state::AppState,
};
use scope_domain::requests::Request;
use std::path::Path;

impl PoolInstall<'_> {
    pub(in crate::git::request_refs) async fn copy_full_scope_packs(
        &self,
        state: &AppState,
        requests: &[Request],
        heads: &[String],
    ) -> Result<usize, ApiError> {
        let PoolRepos::View(view, target) = self.repos else {
            return Ok(0);
        };
        let ids = requests
            .iter()
            .map(|request| request.id.clone())
            .collect::<Vec<_>>();
        let has_full = state
            .metadata
            .repositories()
            .git_pool_packs_for_requests(self.incarnation.repository_id(), &ids)
            .await?
            .into_iter()
            .any(|pack| pack.request_view == *view && pack.scope.view().is_none());
        if !has_full {
            return Ok(0);
        }
        let git = RepositoryGit::load(state, self.incarnation).await?;
        let canonical = match &git.git_head {
            Some(head) => {
                state
                    .repository_engine
                    .materialize_repository(state, self.incarnation, head)
                    .await?
            }
            None => {
                git.view_repo(state, self.config.views(), self.config.views.full())
                    .await?
            }
        };
        let mut installed = 0;
        for request in requests.iter().filter(|request| request.view == *view) {
            let directory = tempfile::Builder::new()
                .prefix("full-scope-")
                .tempdir_in(state.repository_engine.cache_root())
                .map_err(ApiError::internal)?;
            let source = GitRepoHandle::temporary(directory, vec![canonical.share()?])?;
            let repos = request_object_repos(
                state,
                self.incarnation,
                &self.config.object_generation(),
                request.object_author_id(),
            )?;
            {
                let source = source.share()?;
                let repos = repos.clone();
                let canonical_objects = canonical.join("objects");
                blocking::run(move || {
                    super::super::ensure_local_bare_repo(source.as_ref().to_path_buf())?;
                    repos.add_alternates_to(&source)?;
                    crate::git::staging::add_receive_pack_alternate(&source, &canonical_objects)
                })
                .await?;
            }
            let (count, leases) = install_missing_request_packs(
                state,
                &PoolInstall {
                    incarnation: self.incarnation,
                    config: self.config,
                    repos: PoolRepos::Full(&repos, request.object_author_id(), &source),
                },
                std::slice::from_ref(request),
                heads,
            )
            .await?;
            installed += count;
            let source = leases
                .into_iter()
                .fold(source, GitRepoHandle::with_dependency);
            copy_heads(state, source, target, heads).await?;
        }
        Ok(installed)
    }
}

async fn copy_heads(
    state: &AppState,
    source: GitRepoHandle,
    target: &Path,
    heads: &[String],
) -> Result<(), ApiError> {
    let permit = state.runtime_budgets.try_git_materialization()?;
    let target = target.to_path_buf();
    let heads = heads.to_vec();
    blocking::run(move || {
        let _permit = permit;
        let present = present_commits(&source, heads.iter().map(String::as_str))?;
        let retained = present_commits(&target, heads.iter().map(String::as_str))?;
        for head in heads
            .iter()
            .filter(|head| present.contains(*head) && !retained.contains(*head))
        {
            run_git(
                Some(&target),
                &[
                    "fetch",
                    "--no-tags",
                    "--no-write-fetch-head",
                    source.to_string_lossy().as_ref(),
                    &format!("+{head}:refs/scope/revisions/{head}"),
                ],
                "copying authorized revision objects between shelves",
            )?;
        }
        Ok(())
    })
    .await
}
