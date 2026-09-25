use super::{download_snapshot, fetch_bundle_into, request_ref_head, request_ref_oid_is_commit};
use crate::{
    error::ApiError,
    git::{
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::run_git,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, RequestAudience, RequestRevision, canonical_request_ref},
};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static REVISION_BUILD_ATTEMPT: AtomicU64 = AtomicU64::new(1);
const READY_FILE: &str = "scope-request-revision-ready";

/// Callers authorize the request before opening its immutable revision. The
/// cache contains source objects, never permission decisions or rendered data.
pub(crate) async fn with_request_revision_store_repo<T: Send + 'static>(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
    action: impl FnOnce(&Path, &RequestRevision) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    let base_repo = request_base_repo(state.clone(), incarnation.clone(), request.audience);
    with_revision_repo(state, incarnation, request, revision, base_repo, action).await
}

/// The private replica a thin private request snapshot is based on. Public snapshots carry their
/// full history and need no base.
async fn request_base_repo(
    state: AppState,
    incarnation: RepositoryIncarnation,
    audience: RequestAudience,
) -> Result<Option<GitRepoHandle>, ApiError> {
    if audience == RequestAudience::Public {
        return Ok(None);
    }
    let (Some(head), spans) = state
        .metadata
        .repositories()
        .repository_content_source(&incarnation)
        .await?
    else {
        return Ok(None);
    };
    state
        .repository_engine
        .materialize_repository(&state, &incarnation, &head, &spans)
        .await
        .map(Some)
}

/// `base_repo` only runs when the revision is not cached yet.
async fn with_revision_repo<T, Base>(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
    base_repo: impl Future<Output = Result<Option<Base>, ApiError>> + Send + 'static,
    action: impl FnOnce(&Path, &RequestRevision) -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError>
where
    T: Send + 'static,
    Base: AsRef<Path> + Send + 'static,
{
    if revision.request_id != request.id || request.repo_id != incarnation.repository_id() {
        return Err(ApiError::not_found("request revision not found"));
    }
    let key = revision_cache_key(incarnation, request, revision)?;
    let path = state
        .repository_engine
        .cache_root()
        .join(format!("revision-{key}.git"));
    let ready_path = path.join(READY_FILE);
    let state_for_build = state.clone();
    let revision_for_build = revision.clone();
    let request_ref = canonical_request_ref(&request.name);
    let build_path = path.clone();
    let repo = state
        .repository_engine
        .materialize_derived(
            incarnation,
            GitDerivedCacheNamespace::RequestRevision,
            key,
            &path,
            move || ready_path.is_file(),
            move || async move {
                // Resolved before taking a permit, since it may materialize a repository itself.
                let base_repo = base_repo.await?;
                let permit = state_for_build.runtime_budgets.try_git_materialization()?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    build_revision(
                        &build_path,
                        &request_ref,
                        &revision_for_build,
                        base_repo.as_ref().map(AsRef::as_ref),
                        |bundle| {
                            download_snapshot(
                                state_for_build.object_store.as_ref(),
                                &revision_for_build.git_snapshot,
                                bundle,
                            )
                        },
                    )
                })
                .await
                .map_err(|error| {
                    ApiError::internal_message(format!(
                        "request revision materialization task failed: {error}"
                    ))
                })?
            },
        )
        .await?;
    let permit = state.runtime_budgets.wait_git_materialization().await?;
    let revision = revision.clone();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        action(&repo, &revision)
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("request revision inspection task failed: {error}"))
    })?
}

fn revision_cache_key(
    incarnation: &RepositoryIncarnation,
    request: &Request,
    revision: &RequestRevision,
) -> Result<String, ApiError> {
    let identity = serde_json::to_vec(&(
        incarnation,
        &request.id,
        &request.name,
        &revision.id,
        &revision.old_head_oid,
        &revision.new_head_oid,
        &revision.git_snapshot,
    ))
    .map_err(ApiError::internal)?;
    Ok(hex::encode(Sha256::digest(identity)))
}

struct RevisionBuildDirectory(PathBuf);

impl Drop for RevisionBuildDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn build_revision(
    path: &Path,
    request_ref: &str,
    revision: &RequestRevision,
    base_repo: Option<&Path>,
    download: impl FnOnce(&Path) -> Result<(), ApiError>,
) -> Result<(), ApiError> {
    let attempt = REVISION_BUILD_ATTEMPT.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_extension(format!("{}.{}.tmp", std::process::id(), attempt));
    let _cleanup = RevisionBuildDirectory(temporary.clone());
    fs::create_dir(&temporary).map_err(ApiError::internal)?;
    run_git(
        None,
        &["init", "--bare", temporary.to_string_lossy().as_ref()],
        "initializing request revision",
    )?;
    let bundle = temporary.join("revision.bundle");
    download(&bundle)?;
    fetch_bundle_into(
        &temporary,
        request_ref,
        &bundle,
        base_repo,
        "importing request revision",
    )?;
    fs::remove_file(bundle).map_err(ApiError::internal)?;
    if request_ref_head(&temporary, request_ref)?.as_deref() != Some(&revision.new_head_oid)
        || !request_ref_oid_is_commit(&temporary, &revision.new_head_oid)?
    {
        return Err(ApiError::infrastructure_unavailable(
            "request revision snapshot does not contain its expected head",
        ));
    }
    fs::write(temporary.join(READY_FILE), []).map_err(ApiError::internal)?;
    fs::rename(&temporary, path).map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{command::run_git_output, import::git_snapshot_from_ref};
    use scope_domain::requests::{RequestActorRole, RequestAudience};
    use std::sync::{Arc, atomic::AtomicUsize};

    /// A thin revision snapshot on top of the fixture's first commit. The `source` repo under
    /// `root` holds that base.
    fn fixture(root: &Path) -> (RequestRevision, Vec<u8>) {
        let source = root.join("source");
        run_git(
            None,
            &["init", "-b", "topic", source.to_str().unwrap()],
            "create revision fixture",
        )
        .unwrap();
        fs::write(source.join("file.txt"), "base\n").unwrap();
        run_git(Some(&source), &["add", "."], "add fixture").unwrap();
        let commit = || {
            run_git(
                Some(&source),
                &[
                    "-c",
                    "user.name=Scope",
                    "-c",
                    "user.email=scope@example.invalid",
                    "commit",
                    "-am",
                    "fixture",
                ],
                "commit fixture",
            )
            .unwrap()
        };
        commit();
        let old_head_oid = request_ref_head(&source, "refs/heads/topic")
            .unwrap()
            .unwrap();
        fs::write(source.join("file.txt"), "changed\n").unwrap();
        commit();
        let new_head_oid = request_ref_head(&source, "refs/heads/topic")
            .unwrap()
            .unwrap();
        let (git_snapshot, bytes) =
            git_snapshot_from_ref(&source, "refs/heads/topic", Some(&old_head_oid)).unwrap();
        (
            RequestRevision {
                id: "revision".into(),
                request_id: "request".into(),
                position: 1,
                actor_user_id: Some("owner".into()),
                old_head_oid,
                new_head_oid,
                git_snapshot,
                created_at_unix: 1,
            },
            bytes,
        )
    }

    fn request_fixture() -> Request {
        Request {
            id: "request".into(),
            repo_id: "owner/repo".into(),
            name: "topic".into(),
            author_user_id: Some("owner".into()),
            author_role: RequestActorRole::Owner,
            audience: RequestAudience::Private,
            base_main_oid: "a".repeat(40),
            head_oid: "b".repeat(40),
            git_snapshot: None,
            title: "request".into(),
            description_markdown: String::new(),
            activity_version: 1,
            submitted_at_unix: None,
            closed_at_unix: None,
            closed_by_user_id: None,
            merged_at_unix: None,
            merged_by_user_id: None,
            merged_head_oid: None,
            merged_main_oid: None,
            created_at_unix: 1,
            updated_at_unix: 1,
        }
    }

    struct CountingStore {
        inner: Arc<dyn scope_storage::ObjectStore>,
        loads: Arc<AtomicUsize>,
    }
    #[async_trait::async_trait]
    impl scope_storage::ObjectStore for CountingStore {
        async fn put(
            &self,
            key: &str,
            bytes: Vec<u8>,
        ) -> Result<(), scope_storage::ObjectStoreError> {
            self.inner.put(key, bytes).await
        }
        async fn read_to(
            &self,
            key: &str,
            max_bytes: u64,
            output: &mut (dyn tokio::io::AsyncWrite + Send + Unpin),
        ) -> Result<u64, scope_storage::ObjectStoreError> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.inner.read_to(key, max_bytes, output).await
        }
        async fn delete(&self, key: &str) -> Result<(), scope_storage::ObjectStoreError> {
            self.inner.delete(key).await
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_revision_inspections_share_one_verified_import_and_warm_reads_skip_it() {
        let root = tempfile::tempdir().unwrap();
        let (revision, bytes) = fixture(root.path());
        let mut state = AppState::test_state();
        state
            .object_store
            .put(&scope_storage::object_key(&revision.git_snapshot), bytes)
            .await
            .unwrap();
        let incarnation = RepositoryIncarnation::new("owner/repo", "repoi_first").unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        state.object_store = Arc::new(CountingStore {
            inner: state.object_store.clone(),
            loads: loads.clone(),
        });
        for _round in 0..2 {
            let mut readers = Vec::new();
            for _ in 0..10 {
                let state = state.clone();
                let incarnation = incarnation.clone();
                let revision = revision.clone();
                let active = active.clone();
                let peak = peak.clone();
                let base_repo = root.path().join("source");
                readers.push(tokio::spawn(async move {
                    with_revision_repo(
                        &state,
                        &incarnation,
                        &request_fixture(),
                        &revision,
                        async move { Ok(Some(base_repo)) },
                        move |repo, _| {
                            let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                            peak.fetch_max(current, Ordering::SeqCst);
                            let output = run_git_output(
                                Some(repo),
                                &["show", "refs/heads/topic:file.txt"],
                                "inspect revision",
                            )?;
                            assert!(output.status.success());
                            assert_eq!(output.stdout, b"changed\n");
                            active.fetch_sub(1, Ordering::SeqCst);
                            Ok(())
                        },
                    )
                    .await
                    .unwrap();
                }));
            }
            for reader in readers {
                reader.await.unwrap();
            }
            assert_eq!(loads.load(Ordering::SeqCst), 1);
        }
        assert!(peak.load(Ordering::SeqCst) <= 2);
        eprintln!(
            "revision proof: 10 concurrent cold + 10 warm inspections, {} bundle loads/imports, peak {} active inspections",
            loads.load(Ordering::SeqCst),
            peak.load(Ordering::SeqCst)
        );
        fs::remove_dir_all(state.data_dir.as_ref()).unwrap();
    }

    #[test]
    fn failed_revision_import_is_not_published_and_retry_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let (revision, bytes) = fixture(root.path());
        let path = root.path().join("revision.git");
        let source = root.path().join("source");
        let mut wrong_head = revision.clone();
        wrong_head.new_head_oid = wrong_head.old_head_oid.clone();
        assert!(
            build_revision(
                &path,
                "refs/heads/topic",
                &wrong_head,
                Some(&source),
                |bundle| fs::write(bundle, &bytes).map_err(ApiError::internal)
            )
            .is_err()
        );
        assert!(!path.exists());
        assert!(
            !fs::read_dir(root.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
        );
        build_revision(
            &path,
            "refs/heads/topic",
            &revision,
            Some(&source),
            |bundle| fs::write(bundle, &bytes).map_err(ApiError::internal),
        )
        .unwrap();
        assert!(path.join(READY_FILE).is_file());
    }

    #[test]
    fn revision_cache_identity_separates_incarnations_requests_and_snapshot_claims() {
        let request = request_fixture();
        let mut revision = RequestRevision {
            id: "revision".into(),
            request_id: request.id.clone(),
            position: 1,
            actor_user_id: Some("owner".into()),
            old_head_oid: request.base_main_oid.clone(),
            new_head_oid: request.head_oid.clone(),
            git_snapshot: scope_storage::content_object_for_bytes(
                scope_storage::ContentObjectKind::GitBundle,
                b"bundle",
            ),
            created_at_unix: 1,
        };
        let first = RepositoryIncarnation::new("owner/repo", "repoi_first").unwrap();
        let second = RepositoryIncarnation::new("owner/repo", "repoi_second").unwrap();
        let original = revision_cache_key(&first, &request, &revision).unwrap();
        assert_ne!(
            original,
            revision_cache_key(&second, &request, &revision).unwrap()
        );
        let mut other_request = request.clone();
        other_request.id = "another-request".into();
        assert_ne!(
            original,
            revision_cache_key(&first, &other_request, &revision).unwrap()
        );
        revision.git_snapshot.sha256 = "different-content".into();
        assert_ne!(
            original,
            revision_cache_key(&first, &request, &revision).unwrap()
        );
    }
}
