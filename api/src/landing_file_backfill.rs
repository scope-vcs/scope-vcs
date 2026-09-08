use crate::{
    git::content::source_content_bytes, repository_backfill::RepositoryBackfillContext,
    state::AppState,
};
use scope_domain::landing_file::RepositoryLandingFile;

pub async fn backfill_repository_landing_files_for_maintenance(
    database_url: String,
) -> anyhow::Result<usize> {
    let context = RepositoryBackfillContext::from_env(database_url).await?;
    backfill_repository_landing_files(&context).await
}

impl AppState {
    pub async fn backfill_repository_landing_files(&self) -> anyhow::Result<usize> {
        backfill_repository_landing_files(&RepositoryBackfillContext::from_app_state(self)).await
    }
}

async fn backfill_repository_landing_files(
    context: &RepositoryBackfillContext,
) -> anyhow::Result<usize> {
    let candidates = context
        .metadata()
        .repositories()
        .repository_landing_file_backfill_candidates()
        .await?;
    let mut stored = 0;
    for candidate in candidates {
        let git_source = candidate.git_head.as_ref().map(|head| {
            (
                candidate.incarnation.clone(),
                head,
                candidate.git_pack_spans.as_slice(),
            )
        });
        let bytes = source_content_bytes(context, &candidate.blob, git_source)
            .await
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        let landing_file = RepositoryLandingFile::from_source_blob(&candidate.blob, bytes)
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        context
            .metadata()
            .repositories()
            .store_backfilled_repository_landing_file(&candidate.repo_id, landing_file)
            .await?;
        let _ = context
            .delete_repository_cache(&candidate.incarnation)
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        stored += 1;
    }
    let remaining = context
        .metadata()
        .repositories()
        .repository_landing_file_backfill_candidates()
        .await?;
    if !remaining.is_empty() {
        anyhow::bail!(
            "{} repository landing files remain without snapshots",
            remaining.len()
        );
    }
    Ok(stored)
}
