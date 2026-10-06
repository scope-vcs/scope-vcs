use crate::{
    error::ApiError,
    git::{cache::GitRepoHandle, projection_repo::projection_bare_repo_for_state},
    state::AppState,
};
use scope_domain::{
    policy::ScopePath,
    projection::{Projection, ProjectionViewKey},
    repo_config::RepoConfig,
    repository::{
        Repository, RepositoryIncarnation,
        git::{GitHead, GitPackSpan},
    },
    requests::PathHistory,
};
use scope_postgres::db::GitPushContext;

#[derive(Clone, Debug)]
pub(crate) struct RepositoryGit {
    pub(crate) incarnation: RepositoryIncarnation,
    pub(crate) content_version: u64,
    pub(crate) repo_config: RepoConfig,
    pub(crate) git_head: Option<GitHead>,
    pub(crate) git_pack_spans: Vec<GitPackSpan>,
}

impl RepositoryGit {
    pub(crate) fn of_push_context(context: &GitPushContext) -> Self {
        Self {
            incarnation: context.incarnation.clone(),
            content_version: context.content_version,
            repo_config: context.repo_config.clone(),
            git_head: context.git_head.clone(),
            git_pack_spans: context.git_pack_spans.clone(),
        }
    }

    pub(crate) fn of_repository(repo: &Repository) -> Self {
        Self {
            incarnation: repo.incarnation(),
            content_version: repo.record.content_version,
            repo_config: repo.repo_config.clone(),
            git_head: repo.git_head.clone(),
            git_pack_spans: repo.git_pack_spans.clone(),
        }
    }

    pub(crate) async fn projection(
        &self,
        state: &AppState,
        view_key: ProjectionViewKey,
    ) -> Result<Projection, ApiError> {
        Ok(state
            .metadata
            .repositories()
            .repository_projection_source(&self.incarnation, self.content_version)
            .await?
            .project(view_key))
    }

    pub(crate) async fn projection_repo(
        &self,
        state: &AppState,
        projection: &Projection,
    ) -> Result<GitRepoHandle, ApiError> {
        projection_bare_repo_for_state(
            state,
            &self.incarnation,
            projection,
            self.git_head.as_ref(),
            &self.git_pack_spans,
        )
        .await
    }

    pub(crate) async fn view_repo(
        &self,
        state: &AppState,
        view_key: ProjectionViewKey,
    ) -> Result<GitRepoHandle, ApiError> {
        let projection = self.projection(state, view_key).await?;
        self.projection_repo(state, &projection).await
    }

    pub(crate) async fn path_history(
        &self,
        state: &AppState,
        paths: &[ScopePath],
    ) -> Result<PathHistory, ApiError> {
        Ok(state
            .metadata
            .repositories()
            .repository_path_history(&self.incarnation, self.content_version, paths)
            .await?)
    }
}
