use super::{
    PROJECTION_CACHE_ATTEMPT, cached_projection_prefix, projection_bare_repo_for_state,
    projection_bare_repo_with_loader, view_labels,
};
use crate::{error::ApiError, git::content::source_content_bytes_from_repo, state::AppState};
use scope_domain::{
    projection::{Projection, project_graph},
    repository::{Repository, RepositoryIncarnation},
    views::ViewId,
};
use std::{fs, path::Path as FsPath, sync::atomic::Ordering};

pub(crate) async fn verify_and_restore_projection(
    state: &AppState,
    committed: &Repository,
    proposed: Projection,
    native_source_repo: &FsPath,
) -> Result<Option<String>, ApiError> {
    let views = committed.repo_config.views();
    let committed_projection = project_graph(
        &committed.graph,
        &committed.visibility_change_sets,
        views,
        &proposed.view_key,
    );
    let committed_repo = projection_bare_repo_for_state(
        state,
        &committed.incarnation(),
        views,
        &committed_projection,
        committed.git_head.as_ref(),
    )
    .await?;
    let state = state.clone();
    let incarnation = committed.incarnation();
    let labels = view_labels(views, &proposed.view_key);
    let native_source_repo = native_source_repo.to_path_buf();
    let head = crate::git::blocking::run(move || {
        build_proposed_projection(
            &state,
            &incarnation,
            &labels,
            &proposed,
            &native_source_repo,
        )
    })
    .await;
    drop(committed_repo);
    head
}

fn build_proposed_projection(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    labels: &[ViewId],
    projection: &Projection,
    native_source_repo: &FsPath,
) -> Result<Option<String>, ApiError> {
    let prefix =
        cached_projection_prefix(&state.repository_engine, incarnation, labels, projection)?;
    let cache_root = state.repository_engine.cache_root().to_path_buf();
    let attempt = PROJECTION_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let verification_root = cache_root.join(format!(
        ".projection-preflight.{}.{}",
        std::process::id(),
        attempt
    ));
    fs::create_dir_all(&verification_root).map_err(ApiError::internal)?;
    let verification = projection_bare_repo_with_loader(
        &verification_root,
        Some(incarnation),
        labels,
        projection,
        Some(native_source_repo),
        prefix,
        |blob| source_content_bytes_from_repo(state, blob, Some(native_source_repo)),
    )
    .and_then(|repo| {
        let Some(head) = scope_git::projection_head_oid(projection).map_err(ApiError::internal)?
        else {
            return Ok(None);
        };
        let refspec = "+refs/heads/main:refs/scope/internal/stack-view-main";
        crate::git::command::run_git(
            Some(native_source_repo),
            &[
                "fetch",
                "--no-tags",
                repo.to_string_lossy().as_ref(),
                refspec,
            ],
            "restoring prepared view main for stack ancestry",
        )?;
        crate::git::command::run_git(
            Some(native_source_repo),
            &["update-ref", "-d", "refs/scope/internal/stack-view-main"],
            "removing prepared stack view main ref",
        )?;
        Ok(Some(head))
    });
    let cleanup = fs::remove_dir_all(&verification_root).map_err(ApiError::internal);
    match (verification, cleanup) {
        (Ok(head), Ok(())) => Ok(head),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
