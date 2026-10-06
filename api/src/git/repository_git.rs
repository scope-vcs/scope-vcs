use crate::{
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        projection_repo::{cached_projection_repo, projection_bare_repo_for_state},
    },
    state::AppState,
};
use scope_domain::{
    policy::ScopePath,
    projection::Projection,
    repository::{
        Repository, RepositoryIncarnation,
        git::{GitHead, GitPackSpan},
    },
    requests::PathHistory,
    views::{ViewId, Views},
};
use scope_postgres::db::{GitPushContext, GitReadSource};

#[derive(Clone, Debug)]
pub(crate) struct RepositoryGit {
    pub(crate) incarnation: RepositoryIncarnation,
    pub(crate) content_version: u64,
    pub(crate) git_head: Option<GitHead>,
    pub(crate) git_pack_spans: Vec<GitPackSpan>,
}

impl RepositoryGit {
    pub(crate) fn of_push_context(context: &GitPushContext) -> Self {
        Self {
            incarnation: context.incarnation.clone(),
            content_version: context.content_version,
            git_head: context.git_head.clone(),
            git_pack_spans: context.git_pack_spans.clone(),
        }
    }

    pub(crate) fn of_read_source(source: GitReadSource) -> Self {
        Self {
            incarnation: source.context.incarnation(),
            content_version: source.context.record.content_version,
            git_head: source.git_head,
            git_pack_spans: source.git_pack_spans,
        }
    }

    pub(crate) async fn load(
        state: &AppState,
        incarnation: &RepositoryIncarnation,
    ) -> Result<Self, ApiError> {
        let git = state
            .metadata
            .repositories()
            .repository_git_state(incarnation)
            .await?;
        Ok(Self {
            incarnation: incarnation.clone(),
            content_version: git.content_version,
            git_head: git.git_head,
            git_pack_spans: git.git_pack_spans,
        })
    }

    pub(crate) fn of_repository(repo: &Repository) -> Self {
        Self {
            incarnation: repo.incarnation(),
            content_version: repo.record.content_version,
            git_head: repo.git_head.clone(),
            git_pack_spans: repo.git_pack_spans.clone(),
        }
    }

    pub(crate) async fn projection(
        &self,
        state: &AppState,
        views: &Views,
        view: &ViewId,
    ) -> Result<Projection, ApiError> {
        Ok(state
            .metadata
            .repositories()
            .repository_projection_source(&self.incarnation, self.content_version)
            .await?
            .project(views, view))
    }

    pub(crate) async fn view_head(
        &self,
        state: &AppState,
        view: &ViewId,
    ) -> Result<Option<String>, ApiError> {
        Ok(state
            .metadata
            .repositories()
            .repository_view_head(&self.incarnation, self.content_version, view)
            .await?)
    }

    pub(crate) async fn view_repo(
        &self,
        state: &AppState,
        views: &Views,
        view: &ViewId,
    ) -> Result<GitRepoHandle, ApiError> {
        let head_oid = self.view_head(state, view).await?;
        self.view_repo_at(state, views, view, head_oid.as_deref())
            .await
    }

    pub(crate) async fn view_repo_at(
        &self,
        state: &AppState,
        views: &Views,
        view: &ViewId,
        head_oid: Option<&str>,
    ) -> Result<GitRepoHandle, ApiError> {
        if let Some(repo) = cached_projection_repo(state, &self.incarnation, views, view, head_oid)?
        {
            return Ok(repo);
        }
        let projection = self.projection(state, views, view).await?;
        projection_bare_repo_for_state(
            state,
            &self.incarnation,
            &projection,
            self.git_head.as_ref(),
            &self.git_pack_spans,
        )
        .await
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
