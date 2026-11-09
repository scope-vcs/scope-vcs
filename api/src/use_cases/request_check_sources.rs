use crate::{error::ApiError, state::AppState};
use scope_domain::{
    content::SourceBlob,
    repository::RepositoryIncarnation,
    requests::{Request, RequestRevision},
    runs::source::MAX_RUN_SOURCE_BUNDLE_BYTES,
};

pub(crate) struct PreparedRequestRunSource {
    pub(crate) object: SourceBlob,
    fence: scope_postgres::db::ContentRefFence,
}

impl PreparedRequestRunSource {
    pub(crate) async fn finish(
        self,
        state: &AppState,
        result: Result<scope_postgres::db::RequestChecksMutation, ApiError>,
    ) -> Result<scope_postgres::db::RequestChecksMutation, ApiError> {
        if !result
            .as_ref()
            .is_ok_and(|mutation| !mutation.created_runs.is_empty())
        {
            super::content_cleanup::best_effort_cleanup_rollback_source_blobs(
                state,
                std::slice::from_ref(&self.object),
            )
            .await;
        }
        self.fence.release().await;
        result
    }
}

pub(crate) async fn prepare(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
) -> Result<PreparedRequestRunSource, ApiError> {
    let bundle = crate::git::run_source::materialize_request_revision_bundle(
        state,
        incarnation,
        request,
        revision,
        MAX_RUN_SOURCE_BUNDLE_BYTES,
    )
    .await?;
    let bytes = axum::body::to_bytes(bundle.into_body(), MAX_RUN_SOURCE_BUNDLE_BYTES)
        .await
        .map_err(ApiError::internal)?;
    let mut object = scope_storage::content_object_for_bytes(
        scope_storage::ContentObjectKind::GitBundle,
        &bytes,
    );
    object.git_oid = revision.new_head_oid.clone();
    let fence = state
        .metadata
        .acquire_content_ref_fence(std::slice::from_ref(&object.content_ref))
        .await?;
    if let Err(error) = state
        .object_store
        .put(&scope_storage::object_key(&object), bytes.to_vec())
        .await
    {
        fence.release().await;
        return Err(error.into());
    }
    Ok(PreparedRequestRunSource { object, fence })
}

pub(crate) async fn approve(
    state: &AppState,
    repo: &scope_domain::repository::RepoRecord,
    request: &Request,
    actor_user_id: &str,
    reviewed_head_oid: &str,
) -> Result<scope_postgres::db::RequestChecksMutation, ApiError> {
    let now_unix = crate::persistence::unix_now()?;
    let checks = super::request_checks::checks_view(state, repo, request).await?;
    let prepared = if checks
        .evaluation
        .as_ref()
        .is_some_and(|evaluation| evaluation.native_checks().next().is_some())
    {
        let revision = state
            .metadata
            .requests()
            .request_revision_with_head(&request.id, reviewed_head_oid)
            .await?
            .ok_or_else(|| ApiError::conflict("request revision not found"))?;
        Some(prepare(state, &repo.incarnation(), request, &revision).await?)
    } else {
        None
    };
    let result = state
        .metadata
        .requests()
        .approve_request_checks(scope_postgres::db::ApproveRequestChecksCommand {
            request_id: request.id.clone(),
            actor_user_id: actor_user_id.to_string(),
            reviewed_head_oid: reviewed_head_oid.to_string(),
            source: prepared.as_ref().map(|source| source.object.clone()),
            now_unix,
        })
        .await
        .map_err(ApiError::from);
    match prepared {
        Some(source) => source.finish(state, result).await,
        None => result,
    }
}
