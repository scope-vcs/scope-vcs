use super::{
    PROJECTION_CACHE_ATTEMPT, cached_projection_prefix, projection_bare_repo_for_state,
    projection_bare_repo_with_loader, projection_cache_keys, view_labels,
};
use crate::{error::ApiError, git::content::source_content_bytes_from_repo, state::AppState};
use scope_domain::{
    projection::{Projection, project_graph},
    repository::{Repository, RepositoryIncarnation},
    views::ViewId,
};
use std::{
    collections::HashMap,
    fs,
    path::{Path as FsPath, PathBuf},
    sync::{Arc, atomic::Ordering},
};

pub(crate) struct VerifiedProjections {
    root: Arc<VerificationRoot>,
    latest: HashMap<ViewId, VerifiedProjection>,
}

struct VerificationRoot(PathBuf);

impl Drop for VerificationRoot {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            tracing::warn!(%error, path = %self.0.display(), "failed to remove verified projections");
        }
    }
}

struct VerifiedProjection {
    key: String,
    repo: PathBuf,
}

impl VerifiedProjections {
    pub(crate) fn create(state: &AppState) -> Result<Self, ApiError> {
        let attempt = PROJECTION_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
        let root = state.repository_engine.cache_root().join(format!(
            ".projection-preflight.{}.{}",
            std::process::id(),
            attempt
        ));
        fs::create_dir_all(&root).map_err(ApiError::internal)?;
        Ok(Self {
            root: Arc::new(VerificationRoot(root)),
            latest: HashMap::new(),
        })
    }

    pub(crate) async fn verify_and_restore(
        &mut self,
        state: &AppState,
        committed: &Repository,
        proposed: Projection,
        native_source_repo: &FsPath,
    ) -> Result<Option<String>, ApiError> {
        let views = committed.repo_config.views();
        let incarnation = committed.incarnation();
        let labels = view_labels(views, &proposed.view_key);
        let keys = projection_cache_keys(Some(&incarnation), &proposed, &labels)?;
        let extends_latest = self
            .latest
            .get(&proposed.view_key)
            .is_some_and(|latest| keys.contains(&latest.key));
        let committed_repo = if extends_latest {
            None
        } else {
            let committed_projection = project_graph(
                &committed.graph,
                &committed.visibility_change_sets,
                views,
                &proposed.view_key,
            );
            Some(
                projection_bare_repo_for_state(
                    state,
                    &incarnation,
                    views,
                    &committed_projection,
                    committed.git_head.as_ref(),
                )
                .await?,
            )
        };
        let key = keys
            .last()
            .cloned()
            .expect("empty projection has a cache key");
        let view = proposed.view_key.clone();
        let verified = {
            let state = state.clone();
            let root = self.root.clone();
            let native_source_repo = native_source_repo.to_path_buf();
            crate::git::blocking::run(move || {
                build_proposed_projection(
                    &state,
                    &incarnation,
                    &labels,
                    &proposed,
                    &native_source_repo,
                    &root.0,
                )
            })
            .await
        };
        drop(committed_repo);
        let (head, repo) = verified?;
        if let Some(previous) = self.latest.insert(view, VerifiedProjection { key, repo })
            && !self
                .latest
                .values()
                .any(|latest| latest.repo == previous.repo)
        {
            fs::remove_dir_all(&previous.repo).map_err(ApiError::internal)?;
        }
        Ok(head)
    }
}

fn build_proposed_projection(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    labels: &[ViewId],
    projection: &Projection,
    native_source_repo: &FsPath,
    verified_root: &FsPath,
) -> Result<(Option<String>, PathBuf), ApiError> {
    let prefix = cached_projection_prefix(
        &state.repository_engine,
        incarnation,
        labels,
        projection,
        Some(verified_root),
    )?;
    let repo = projection_bare_repo_with_loader(
        verified_root,
        Some(incarnation),
        labels,
        projection,
        Some(native_source_repo),
        prefix,
        |blob| source_content_bytes_from_repo(state, blob, Some(native_source_repo)),
    )?;
    let Some(head) = scope_git::projection_head_oid(projection).map_err(ApiError::internal)? else {
        return Ok((None, repo));
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
    Ok((Some(head), repo))
}
