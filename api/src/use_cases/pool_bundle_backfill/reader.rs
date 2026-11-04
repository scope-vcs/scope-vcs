use crate::{
    error::ApiError,
    git::{
        blocking::block_on as wait,
        cache::GitRepoHandle,
        command::run_git,
        pool_install::install_chain,
        repository_git::RepositoryGit,
        request_refs::{
            PoolInstall, PoolRepos, RequestObjectRepos, ensure_local_bare_repo,
            install_missing_request_packs, shares_objects, view_object_repos,
        },
        staging::add_receive_pack_alternate,
    },
    state::AppState,
};
use scope_domain::{
    repo_config::RepoConfig,
    repository::git::{GitPoolPack, GitPoolPackOwner},
    requests::Request,
    views::ViewId,
};
use std::path::PathBuf;

pub(crate) struct PoolBundleReader<'a> {
    pub(crate) directory: tempfile::TempDir,
    pub(crate) path: PathBuf,
    git: &'a RepositoryGit,
    config: &'a RepoConfig,
    actor: &'a str,
    view: &'a ViewId,
    targets: Option<RequestObjectRepos>,
    leases: Vec<GitRepoHandle>,
}

impl<'a> PoolBundleReader<'a> {
    pub(crate) fn new(
        state: &AppState,
        git: &'a RepositoryGit,
        config: &'a RepoConfig,
        actor: &'a str,
        view: &'a ViewId,
    ) -> Result<Self, ApiError> {
        let directory =
            tempfile::tempdir_in(state.data_dir.as_ref()).map_err(ApiError::internal)?;
        let path = ensure_local_bare_repo(directory.path().join("rev.git"))?;
        let mut leases = Vec::new();
        let targets = if view == config.views.full() {
            let base = match &git.git_head {
                Some(head) => wait(state.repository_engine.materialize_repository(
                    state,
                    &git.incarnation,
                    head,
                ))?,
                None => wait(git.view_repo(state, config.views(), view))?,
            };
            let canonical = state.repository_engine.repository_path(&git.incarnation);
            let targets = RequestObjectRepos::initialize(
                directory.path(),
                &config.object_generation(),
                actor,
                &canonical,
            )?;
            targets.add_alternates_to(&path)?;
            add_receive_pack_alternate(&path, &base.join("objects"))?;
            leases.push(base);
            Some(targets)
        } else {
            let base = wait(view_object_repos(state, &git.incarnation, config, view))?;
            add_receive_pack_alternate(&path, &base.main.join("objects"))?;
            leases.push(base.main);
            None
        };
        Ok(Self {
            directory,
            path,
            git,
            config,
            actor,
            view,
            targets,
            leases,
        })
    }

    pub(crate) fn install(
        &mut self,
        state: &AppState,
        requests: &[Request],
        heads: &[String],
    ) -> Result<(), ApiError> {
        let install = PoolInstall {
            incarnation: &self.git.incarnation,
            config: self.config,
            repos: self.repos(),
        };
        let (_, leases) = wait(install_missing_request_packs(
            state, &install, requests, heads,
        ))?;
        self.leases.extend(leases);
        Ok(())
    }

    pub(crate) fn lease_candidate_base(
        &mut self,
        state: &AppState,
        pack: &GitPoolPack,
    ) -> Result<(), ApiError> {
        let GitPoolPackOwner::Request { scope, .. } = &pack.owner else {
            return Err(ApiError::internal_message(
                "backfill requires a request pack",
            ));
        };
        if let Some(view) = scope.view() {
            let install = PoolInstall {
                incarnation: &self.git.incarnation,
                config: self.config,
                repos: self.repos(),
            };
            let lease = wait(install.lease_view_base(state, view))?;
            self.leases.extend(lease);
        }
        Ok(())
    }

    pub(crate) fn install_candidate(
        &mut self,
        state: &AppState,
        request: &Request,
        pack: &GitPoolPack,
    ) -> Result<(), ApiError> {
        let GitPoolPackOwner::Request { scope, .. } = &pack.owner else {
            return Err(ApiError::internal_message(
                "backfill requires a request pack",
            ));
        };
        self.install(state, std::slice::from_ref(request), &pack.prerequisites)?;
        if let Some(target) = self.repos().pack_target(
            scope,
            shares_objects(request, self.config),
            request.is_author(self.actor),
        ) {
            wait(install_chain(
                state,
                &self.git.incarnation,
                target,
                vec![pack.clone()],
            ))?;
        }
        Ok(())
    }

    fn repos(&self) -> PoolRepos<'_> {
        match &self.targets {
            Some(targets) => PoolRepos::Full(targets, self.actor, &self.path),
            None => PoolRepos::View(self.view, &self.path),
        }
    }

    pub(crate) fn verify(&self, head: &str) -> Result<(), ApiError> {
        run_git(
            Some(&self.path),
            &["fsck", "--connectivity-only", head],
            "verify connectivity",
        )
    }
}
