#[cfg(test)]
use crate::git::import::run_git_output;
use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::GitContext,
    git::{
        cache::{
            GitDerivedCacheCoordinator, GitDerivedCacheNamespace, GitRepoHandle,
            RepositoryGitCache, sanitize_repository_git_cache_repo,
        },
        import::run_git,
        restore::{
            index_git_pack, restore_git_pack_spans, run_timed_git_restore_phase,
            run_timed_git_restore_phase_async,
        },
    },
};
use scope_domain::repository::{
    RepositoryIncarnation,
    git::{GitHead, GitPackSpan, validate_git_pack_layout},
};
use scope_git_process::{ProcessLimits, run_with_stdin_reader};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static REPOSITORY_MATERIALIZATION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const MATERIALIZATION_PATH_HIT: u8 = 0;
const MATERIALIZATION_PATH_CATCH_UP: u8 = 1;
const MATERIALIZATION_PATH_RESTORE: u8 = 2;

/// Owns this API process's disposable Git replicas and coordinates mutations of
/// each replica through one repository-scoped stream. Durable publication is
/// still ordered by the Postgres repository aggregate, and compaction remains a
/// worker concern; local promotion receives only an already-committed frontier.
pub(crate) struct RepositoryEngine {
    cache: Arc<RepositoryGitCache>,
    materializations: Arc<GitDerivedCacheCoordinator>,
}

impl RepositoryEngine {
    pub(crate) fn new(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        Ok(Arc::new(Self {
            cache: RepositoryGitCache::new(root, max_bytes)?,
            materializations: Arc::new(GitDerivedCacheCoordinator::default()),
        }))
    }

    pub(crate) fn cache_root(&self) -> &Path {
        self.cache.root()
    }

    #[cfg(test)]
    pub(crate) fn repository_path(&self, incarnation: &RepositoryIncarnation) -> PathBuf {
        self.cache.path_for(incarnation)
    }

    pub(crate) fn delete_repository_cache(
        &self,
        incarnation: &RepositoryIncarnation,
    ) -> Result<bool, ApiError> {
        self.cache.remove(incarnation)
    }

    pub(crate) fn lease_derived(
        self: &Arc<Self>,
        path: PathBuf,
    ) -> Result<GitRepoHandle, ApiError> {
        self.cache.lease_derived(path)
    }

    /// Coalesces immutable derived views by their content-derived key. These
    /// views do not participate in the repository replica's mutation stream.
    pub(crate) async fn materialize_derived<IsReady, Build, BuildFuture>(
        self: &Arc<Self>,
        incarnation: &RepositoryIncarnation,
        namespace: GitDerivedCacheNamespace,
        key: String,
        path: &Path,
        is_ready: IsReady,
        build: Build,
    ) -> Result<GitRepoHandle, ApiError>
    where
        IsReady: Fn() -> bool + Send + Sync + 'static,
        Build: FnOnce() -> BuildFuture + Send + 'static,
        BuildFuture: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        let repo = self.cache.lease_derived(path.to_path_buf())?;
        let build_lease = self.cache.lease_derived(path.to_path_buf())?;
        let started_at = Instant::now();
        let cache_hit = is_ready();
        let built = Arc::new(AtomicBool::new(false));
        let built_for_build = built.clone();
        let cache_for_build = self.cache.clone();
        let result = self
            .materializations
            .materialize_async(namespace, key, is_ready, move || async move {
                let _build_lease = build_lease;
                built_for_build.store(true, Ordering::Relaxed);
                let result = build().await;
                if result.is_ok() {
                    let pruned = tokio::task::spawn_blocking(move || cache_for_build.prune()).await;
                    match pruned {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => tracing::warn!(error = %error.operator_diagnostic(), "failed to prune derived Git caches"),
                        Err(error) => tracing::warn!(%error, "derived Git cache pruning task failed"),
                    }
                }
                result
            })
            .await
            .map(|()| repo);
        tracing::info!(
            repository_id = incarnation.repository_id(),
            repository_incarnation_id = incarnation.incarnation_id(),
            namespace = ?namespace,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            success = result.is_ok(),
            "repository derived Git materialization completed"
        );
        result
    }

    /// Opens the local replica at or beyond the requested durable frontier,
    /// serializing any repair or catch-up with post-push replica updates.
    pub(crate) async fn materialize_repository<C: GitContext>(
        self: &Arc<Self>,
        context: &C,
        incarnation: &RepositoryIncarnation,
        head: &GitHead,
        pack_spans: &[GitPackSpan],
    ) -> Result<GitRepoHandle, ApiError> {
        let repository_id = incarnation.repository_id();
        let repo = self.cache.lease(incarnation)?;
        let repo_path = repo.as_ref().to_path_buf();
        let cache_root = self.cache_root();
        let cache_for_ready = self.cache.clone();
        let incarnation_for_ready = incarnation.clone();
        let repo_path_for_ready = repo_path.clone();
        let requested_sequence = head.push_sequence;
        let is_ready = move || {
            repository_cache_is_ready(&repo_path_for_ready)
                && cache_for_ready
                    .applied_sequence(&incarnation_for_ready, &repo_path_for_ready)
                    .is_some_and(|applied| applied >= requested_sequence)
        };
        let started_at = Instant::now();
        let applied_before = self.cache.applied_sequence(incarnation, &repo_path);
        let cache_hit = is_ready();
        let built = Arc::new(AtomicBool::new(false));
        let materialization_path = Arc::new(AtomicU8::new(MATERIALIZATION_PATH_HIT));
        let engine_for_build = self.clone();
        let context_for_build = context.clone();
        let incarnation_for_build = incarnation.clone();
        let head_for_build = head.clone();
        let pack_spans_for_build = pack_spans.to_vec();
        let repo_path_for_build = repo_path.clone();
        let cache_root_for_build = cache_root.to_path_buf();
        let repository_id_for_build = repository_id.to_string();
        let built_for_build = built.clone();
        let materialization_path_for_build = materialization_path.clone();
        // The detached build must retain its own lease if the requesting
        // future drops the handle returned above while Git is still working.
        let build_repo_lease = self.cache.lease(incarnation)?;
        let result = self.coordinate_repository(incarnation, is_ready, move || async move {
            let _build_repo_lease = build_repo_lease;
            built_for_build.store(true, Ordering::Relaxed);
            let _permit = context_for_build
                .runtime_budgets()
                .try_git_materialization()?;
            match engine_for_build
                .cache
                .applied_sequence(&incarnation_for_build, &repo_path_for_build)
            {
                Some(applied)
                    if applied < head_for_build.push_sequence
                        && repo_path_for_build.is_dir() =>
                {
                    materialization_path_for_build
                        .store(MATERIALIZATION_PATH_CATCH_UP, Ordering::Relaxed);
                    engine_for_build
                        .catch_up(
                            &context_for_build,
                            &repository_id_for_build,
                            &head_for_build,
                            &pack_spans_for_build,
                            applied,
                            &repo_path_for_build,
                        )
                        .await?;
                    engine_for_build.cache.note_applied(
                        &incarnation_for_build,
                        &repo_path_for_build,
                        head_for_build.push_sequence,
                    )
                }
                Some(applied) if applied == head_for_build.push_sequence
                    && repository_cache_is_ready(&repo_path_for_build) =>
                {
                    Ok(())
                }
                // Replicas are monotonic. A reader with an older database
                // frontier may safely use the newer local object set.
                Some(applied) if applied > head_for_build.push_sequence
                    && repository_cache_is_ready(&repo_path_for_build) =>
                {
                    Ok(())
                }
                _ => {
                    materialization_path_for_build
                        .store(MATERIALIZATION_PATH_RESTORE, Ordering::Relaxed);
                    let attempt =
                        REPOSITORY_MATERIALIZATION_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                    let temp_path = cache_root_for_build.join(format!(
                        "repo-materializing.{}.{}.tmp",
                        std::process::id(),
                        attempt
                    ));
                    if let Err(error) = restore_git_pack_spans(
                        &context_for_build,
                        &repository_id_for_build,
                        &head_for_build,
                        &pack_spans_for_build,
                        &temp_path,
                        None,
                    )
                    .await
                    {
                        let _ = fs::remove_dir_all(&temp_path);
                        return Err(error);
                    }
                    engine_for_build.cache.note_applied(
                        &incarnation_for_build,
                        &temp_path,
                        head_for_build.push_sequence,
                    )?;
                    if repo_path_for_build.exists()
                        && let Err(error) = fs::remove_dir_all(&repo_path_for_build)
                    {
                        let _ = fs::remove_dir_all(&temp_path);
                        return Err(ApiError::internal(error));
                    }
                    match fs::rename(&temp_path, &repo_path_for_build) {
                        Ok(()) => Ok(()),
                        Err(error)
                            if repository_cache_is_ready(&repo_path_for_build)
                                && engine_for_build
                                    .cache
                                    .applied_sequence(
                                        &incarnation_for_build,
                                        &repo_path_for_build,
                                    )
                                    .is_some_and(|applied| {
                                        applied >= head_for_build.push_sequence
                                    }) =>
                        {
                            let _ = fs::remove_dir_all(&temp_path);
                            tracing::debug!(%error, path = %repo_path_for_build.display(), "using externally-created repository Git cache");
                            Ok(())
                        }
                        Err(error) => {
                            let _ = fs::remove_dir_all(&temp_path);
                            Err(ApiError::internal(error))
                        }
                    }
                }
            }
        })
        .await;
        tracing::info!(
            repository_id,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            materialization_path = materialization_path_name(
                materialization_path.load(Ordering::Relaxed),
                cache_hit,
                built.load(Ordering::Relaxed),
            ),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = head.push_sequence,
            applied_sequence_before = applied_before,
            applied_sequence_after = self.cache.applied_sequence(incarnation, &repo_path),
            pack_span_count = pack_spans.len(),
            total_pack_bytes = pack_spans
                .iter()
                .map(|span| span.segment.plaintext_bytes)
                .sum::<u64>(),
            success = result.is_ok(),
            "repository Git replica materialization completed"
        );
        result?;
        Ok(repo)
    }

    pub(crate) fn sync_after_push(
        &self,
        incarnation: &RepositoryIncarnation,
        local_pack: &Path,
        expected_head: &str,
        push_sequence: u64,
    ) -> Result<(), ApiError> {
        // Post-commit synchronization mutates the same disposable replica as
        // readers. Keep it leased so the periodic cache reaper cannot remove it
        // while Git is replacing refs or pack files.
        let repository_id = incarnation.repository_id();
        let repo = self.cache.lease(incarnation)?;
        let target = repo.as_ref().to_path_buf();
        let is_ready = || {
            self.cache
                .applied_sequence(incarnation, &target)
                .is_some_and(|applied| applied >= push_sequence)
                && target.is_dir()
        };
        let started_at = Instant::now();
        let cache_hit = is_ready();
        let built = AtomicBool::new(false);
        let result = self.coordinate_repository_blocking(incarnation, is_ready, || {
            built.store(true, Ordering::Relaxed);
            if target.is_dir() {
                index_local_pack(&target, local_pack)?;
                run_timed_git_restore_phase(
                    repository_id,
                    "promote_update_ref",
                    Some(&target),
                    &[
                        "update-ref",
                        &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                        expected_head,
                    ],
                    "advancing repository Git cache from accepted segment",
                )?;
            } else if push_sequence > 1 {
                // Later segments exclude objects reachable from the previous head.
                // A reader can rebuild the absent cache from the durable pack layout.
                return Err(ApiError::internal_message(
                    "incremental Git segment cannot seed a missing repository cache",
                ));
            } else {
                let attempt = REPOSITORY_MATERIALIZATION_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                let temp = self.cache_root().join(format!(
                    "repo-promoting.{}.{}.tmp",
                    std::process::id(),
                    attempt
                ));
                run_git(
                    None,
                    &["--bare", "init", temp.to_string_lossy().as_ref()],
                    "seeding repository Git cache from accepted push",
                )?;
                let build = (|| {
                    index_local_pack(&temp, local_pack)?;
                    run_timed_git_restore_phase(
                        repository_id,
                        "promote_update_ref",
                        Some(&temp),
                        &[
                            "update-ref",
                            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                            expected_head,
                        ],
                        "seeding repository Git cache head",
                    )?;
                    sanitize_repository_git_cache_repo(&temp, expected_head)?;
                    self.cache.note_applied(incarnation, &temp, push_sequence)?;
                    fs::rename(&temp, &target).map_err(ApiError::internal)
                })();
                if build.is_err() {
                    let _ = fs::remove_dir_all(&temp);
                }
                return build;
            }
            sanitize_repository_git_cache_repo(&target, expected_head)?;
            self.cache.note_applied(incarnation, &target, push_sequence)
        });
        tracing::info!(
            repository_id,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = push_sequence,
            applied_sequence = self.cache.applied_sequence(incarnation, &target),
            success = result.is_ok(),
            "repository Git replica post-push synchronization completed"
        );
        result
    }

    pub(crate) fn start_reaper(self: &Arc<Self>) {
        let engine = self.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5 * 60));
            loop {
                interval.tick().await;
                let cache = engine.cache.clone();
                if let Err(error) = crate::git::blocking::run(move || cache.prune()).await {
                    tracing::warn!(error = %error.operator_diagnostic(), "failed to prune local repository Git caches");
                }
            }
        });
    }

    async fn coordinate_repository<IsReady, Operation, OperationFuture>(
        self: &Arc<Self>,
        incarnation: &RepositoryIncarnation,
        is_ready: IsReady,
        operation: Operation,
    ) -> Result<(), ApiError>
    where
        IsReady: Fn() -> bool + Send + Sync + 'static,
        Operation: FnOnce() -> OperationFuture + Send + 'static,
        OperationFuture: std::future::Future<Output = Result<(), ApiError>> + Send + 'static,
    {
        self.materializations
            .materialize_async(
                GitDerivedCacheNamespace::Repository,
                format!(
                    "{}:{}{}",
                    incarnation.repository_id().len(),
                    incarnation.repository_id(),
                    incarnation.incarnation_id()
                ),
                is_ready,
                operation,
            )
            .await
    }

    fn coordinate_repository_blocking(
        &self,
        incarnation: &RepositoryIncarnation,
        is_ready: impl Fn() -> bool,
        operation: impl FnOnce() -> Result<(), ApiError>,
    ) -> Result<(), ApiError> {
        self.materializations.materialize(
            GitDerivedCacheNamespace::Repository,
            format!(
                "{}:{}{}",
                incarnation.repository_id().len(),
                incarnation.repository_id(),
                incarnation.incarnation_id()
            ),
            is_ready,
            operation,
        )
    }

    async fn catch_up<C: GitContext>(
        &self,
        context: &C,
        repository_id: &str,
        head: &GitHead,
        pack_spans: &[GitPackSpan],
        applied_sequence: u64,
        repo_root: &Path,
    ) -> Result<(), ApiError> {
        validate_git_pack_layout(pack_spans)
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        if applied_sequence >= head.push_sequence {
            return Err(ApiError::internal_message(
                "repository Git cache sequence cannot catch up to an older head",
            ));
        }
        let next_sequence = applied_sequence.saturating_add(1);
        let missing = pack_spans
            .iter()
            .skip_while(|span| span.last_sequence < next_sequence)
            .collect::<Vec<_>>();
        let first = missing.first().ok_or_else(|| {
            ApiError::internal_message("repository Git cache has no pack span for its missing tail")
        })?;
        if first.first_sequence > next_sequence {
            return Err(ApiError::internal_message(
                "repository Git cache missing tail starts after the required sequence",
            ));
        }
        let missing_count = missing.len();
        for (index, span) in missing.into_iter().enumerate() {
            index_git_pack(
                context,
                repo_root,
                repository_id,
                span,
                (index + 1, missing_count),
                None,
            )
            .await?;
        }
        run_timed_git_restore_phase_async(
            repository_id,
            "update_ref",
            Some(repo_root.to_path_buf()),
            vec![
                "update-ref".to_string(),
                format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                head.head_oid.clone(),
            ],
            "advancing repository Git cache head",
            None,
        )
        .await?;
        run_timed_git_restore_phase_async(
            repository_id,
            "fsck",
            Some(repo_root.to_path_buf()),
            vec![
                "fsck".to_string(),
                "--connectivity-only".to_string(),
                head.head_oid.clone(),
            ],
            "verifying caught-up repository Git cache",
            None,
        )
        .await
    }
}

fn index_local_pack(repo_root: &Path, local_pack: &Path) -> Result<(), ApiError> {
    let pack = fs::File::open(local_pack).map_err(ApiError::internal)?;
    let output = run_with_stdin_reader(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_root)
            .args(["index-pack", "--stdin"]),
        pack,
        ProcessLimits::new(crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout()),
        "indexing accepted local Git segment",
    )
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(ApiError::infrastructure_unavailable(format!(
            "indexing accepted local Git segment: {}",
            crate::git::upload::truncated_git_stderr(&output.stderr).trim()
        )))
    }
}

fn repository_cache_is_ready(repo_path: &Path) -> bool {
    repo_path.is_dir() && repo_path.join("objects").is_dir()
}

fn materialization_outcome(cache_hit: bool, built: bool) -> &'static str {
    if cache_hit {
        "hit"
    } else if built {
        "build"
    } else {
        "wait"
    }
}

fn materialization_path_name(path: u8, cache_hit: bool, built: bool) -> &'static str {
    if cache_hit {
        return "hit";
    }
    if !built {
        return "wait";
    }
    match path {
        MATERIALIZATION_PATH_CATCH_UP => "catch_up",
        MATERIALIZATION_PATH_RESTORE => "restore",
        _ => "hit",
    }
}

#[cfg(test)]
mod tests;
