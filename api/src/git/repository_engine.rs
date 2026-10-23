mod revision;
pub(crate) use revision::GitRevision;

use crate::background_work::{BackgroundLoop, BackgroundWork, Cadence, Readiness};
#[cfg(test)]
use crate::git::command::run_git_output;
use crate::{
    error::ApiError,
    git::{
        cache::{
            GitDerivedCacheCoordinator, GitDerivedCacheNamespace, GitRepoHandle,
            RepositoryGitCache, sanitize_repository_git_cache_repo,
        },
        command::run_git,
        pool_install::{install_main_head, present_commits},
        request_refs::UnreadableSnapshots,
        restore::{
            advance_head_and_verify, install_verified_git_pack, restore_main_head,
            run_timed_git_restore_phase,
        },
    },
    state::AppState,
};
use scope_domain::repository::{RepositoryIncarnation, git::GitHead};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::time::MissedTickBehavior;

static REPOSITORY_MATERIALIZATION_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const MATERIALIZATION_PATH_HIT: u8 = 0;
const MATERIALIZATION_PATH_CATCH_UP: u8 = 1;
const MATERIALIZATION_PATH_RESTORE: u8 = 2;

pub(crate) struct PushedPack<'a> {
    pub(crate) path: &'a Path,
    pub(crate) prerequisites: &'a [String],
}

#[derive(Debug, PartialEq, Eq)]
enum PushedPackLink {
    Linked,
    MissingPrerequisites,
}

pub(crate) struct RepositoryEngine {
    cache: Arc<RepositoryGitCache>,
    materializations: Arc<GitDerivedCacheCoordinator>,
    packs: Arc<scope_storage::GitSegmentStore>,
    unreadable_snapshots: UnreadableSnapshots,
    max_bytes: u64,
}

impl RepositoryEngine {
    pub(crate) fn new(
        root: PathBuf,
        max_bytes: usize,
        packs: Arc<scope_storage::GitSegmentStore>,
    ) -> Result<Arc<Self>, ApiError> {
        Ok(Arc::new(Self {
            cache: RepositoryGitCache::new(root, max_bytes)?,
            materializations: Arc::new(GitDerivedCacheCoordinator::default()),
            packs,
            unreadable_snapshots: UnreadableSnapshots::default(),
            max_bytes: max_bytes as u64,
        }))
    }

    #[cfg(test)]
    pub(crate) fn for_tests(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        use scope_storage::{EncryptionKey, GitSegmentStore, GitSegmentStoreConfig, MemoryBackend};
        let packs = GitSegmentStore::new(
            Arc::new(MemoryBackend::default()),
            EncryptionKey::new("test", [9; 32]).unwrap(),
            GitSegmentStoreConfig::new(root.join(".segments")),
        )
        .unwrap();
        Self::new(root, max_bytes, Arc::new(packs))
    }

    fn prune(&self) -> Result<(), ApiError> {
        let repo_bytes = self.cache.usage_bytes()?;
        self.packs
            .evict_verified_cache(self.max_bytes.saturating_sub(repo_bytes))
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        let pack_bytes = self
            .packs
            .verified_cache_usage()
            .map_err(|error| ApiError::internal_message(error.to_string()))?;
        self.cache
            .prune_to(self.max_bytes.saturating_sub(pack_bytes.retained_bytes))
    }

    pub(crate) fn unreadable_snapshots(&self) -> &UnreadableSnapshots {
        &self.unreadable_snapshots
    }

    pub(crate) fn cache_root(&self) -> &Path {
        self.cache.root()
    }

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
        let engine_for_build = self.clone();
        let result = self
            .materializations
            .materialize_async(namespace, key, is_ready, move || async move {
                let _build_lease = build_lease;
                built_for_build.store(true, Ordering::Relaxed);
                let result = build().await;
                if result.is_ok() {
                    let pruned = tokio::task::spawn_blocking(move || engine_for_build.prune()).await;
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

    pub(crate) async fn materialize_repository(
        self: &Arc<Self>,
        state: &AppState,
        incarnation: &RepositoryIncarnation,
        head: &GitHead,
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
        let state_for_build = state.clone();
        let incarnation_for_build = incarnation.clone();
        let head_for_build = head.clone();
        let repo_path_for_build = repo_path.clone();
        let cache_root_for_build = cache_root.to_path_buf();
        let built_for_build = built.clone();
        let materialization_path_for_build = materialization_path.clone();
        let build_repo_lease = self.cache.lease(incarnation)?;
        let result = self.coordinate_repository(incarnation, is_ready, move || async move {
            let _build_repo_lease = build_repo_lease;
            built_for_build.store(true, Ordering::Relaxed);
            let _permit = state_for_build.runtime_budgets.try_git_materialization()?;
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
                            &state_for_build,
                            &incarnation_for_build,
                            &head_for_build,
                            &repo_path_for_build,
                        )
                        .await?;
                    engine_for_build.cache.note_applied(
                        &incarnation_for_build,
                        &repo_path_for_build,
                        head_for_build.push_sequence,
                    )
                }
                Some(applied) if applied >= head_for_build.push_sequence
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
                    if let Err(error) = restore_main_head(
                        &state_for_build,
                        &incarnation_for_build,
                        &head_for_build,
                        &temp_path,
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
            materialization_path = build_path_name(materialization_path.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = head.push_sequence,
            applied_sequence_before = applied_before,
            applied_sequence_after = self.cache.applied_sequence(incarnation, &repo_path),
            success = result.is_ok(),
            "repository Git replica materialization completed"
        );
        result?;
        if built.load(Ordering::Relaxed) {
            let engine = self.clone();
            match tokio::task::spawn_blocking(move || engine.prune()).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(error = %error.operator_diagnostic(), "Git cache pruning failed")
                }
                Err(error) => tracing::warn!(%error, "Git cache pruning task failed"),
            }
        }
        Ok(repo)
    }

    pub(crate) async fn sync_after_push(
        self: &Arc<Self>,
        state: &AppState,
        incarnation: &RepositoryIncarnation,
        pushed: PushedPack<'_>,
        head: &GitHead,
    ) -> Result<(), ApiError> {
        let link = {
            let engine = self.clone();
            let incarnation = incarnation.clone();
            let pack = pushed.path.to_path_buf();
            let prerequisites = pushed.prerequisites.to_vec();
            let head = head.clone();
            crate::git::blocking::run(move || {
                engine.link_pushed_pack(&incarnation, &pack, &prerequisites, &head)
            })
            .await?
        };
        match link {
            PushedPackLink::Linked => Ok(()),
            PushedPackLink::MissingPrerequisites => self
                .materialize_repository(state, incarnation, head)
                .await
                .map(drop),
        }
    }

    fn link_pushed_pack(
        &self,
        incarnation: &RepositoryIncarnation,
        local_pack: &Path,
        prerequisites: &[String],
        head: &GitHead,
    ) -> Result<PushedPackLink, ApiError> {
        let repository_id = incarnation.repository_id();
        let expected_head = head.head_oid.as_str();
        let push_sequence = head.push_sequence;
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
        let missing_prerequisites = AtomicBool::new(false);
        let result = self.coordinate_repository_blocking(incarnation, is_ready, || {
            built.store(true, Ordering::Relaxed);
            let present = present_commits(&target, prerequisites.iter().map(String::as_str))?;
            if !prerequisites.iter().all(|oid| present.contains(oid)) {
                missing_prerequisites.store(true, Ordering::Relaxed);
                return Ok(());
            }
            if target.is_dir() {
                install_verified_git_pack(
                    &target,
                    local_pack,
                    crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout(),
                )?;
                run_timed_git_restore_phase(
                    repository_id,
                    "promote_update_ref",
                    Some(&target),
                    &[
                        "update-ref",
                        &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
                        expected_head,
                    ],
                    "advancing repository Git cache from the accepted pack",
                )?;
                sanitize_repository_git_cache_repo(&target, expected_head)?;
                return self.cache.note_applied(incarnation, &target, push_sequence);
            }
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
                install_verified_git_pack(
                    &temp,
                    local_pack,
                    crate::runtime_budgets::RuntimeBudgets::default_git_command_timeout(),
                )?;
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
            build
        });
        let missing_prerequisites = missing_prerequisites.load(Ordering::Relaxed);
        tracing::info!(
            repository_id,
            cache_outcome = materialization_outcome(cache_hit, built.load(Ordering::Relaxed)),
            elapsed_us = started_at.elapsed().as_micros(),
            requested_sequence = push_sequence,
            applied_sequence = self.cache.applied_sequence(incarnation, &target),
            missing_prerequisites,
            success = result.is_ok(),
            "repository Git replica post-push synchronization completed"
        );
        result.map(|()| {
            if missing_prerequisites {
                PushedPackLink::MissingPrerequisites
            } else {
                PushedPackLink::Linked
            }
        })
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
                repository_coordination_key(incarnation),
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
            repository_coordination_key(incarnation),
            is_ready,
            operation,
        )
    }

    async fn catch_up(
        &self,
        state: &AppState,
        incarnation: &RepositoryIncarnation,
        head: &GitHead,
        repo_root: &Path,
    ) -> Result<(), ApiError> {
        install_main_head(state, incarnation, repo_root, head).await?;
        advance_head_and_verify(
            incarnation.repository_id(),
            repo_root,
            &head.head_oid,
            "advancing repository Git cache head",
            "verifying caught-up repository Git cache",
        )
        .await
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

fn build_path_name(path: u8) -> Option<&'static str> {
    match path {
        MATERIALIZATION_PATH_CATCH_UP => Some("catch_up"),
        MATERIALIZATION_PATH_RESTORE => Some("restore"),
        _ => None,
    }
}

fn repository_coordination_key(incarnation: &RepositoryIncarnation) -> String {
    format!(
        "{}:{}{}",
        incarnation.repository_id().len(),
        incarnation.repository_id(),
        incarnation.incarnation_id()
    )
}

impl BackgroundWork {
    pub(crate) fn start_repository_cache_pruning(&mut self) {
        self.spawn(
            BackgroundLoop {
                name: "local repository Git cache pruning",
                cadence: Cadence::Fixed {
                    period: Duration::from_secs(5 * 60),
                    missed_ticks: MissedTickBehavior::Burst,
                },
                readiness: Readiness::Ungated,
            },
            |state| async move { state.repository_engine.prune() },
        );
    }
}

#[cfg(test)]
mod tests;
