use super::*;
use crate::git::request_refs::request_ref_head;
use crate::git::{command::run_git_output, import::git_snapshot_from_ref};
use scope_domain::requests::RequestActorRole;
use scope_domain::views::ViewId;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

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
            base_version: 1,
            id: "revision".into(),
            request_id: "request".into(),
            position: 1,
            actor_user_id: Some("owner".into()),
            base: scope_domain::requests::RequestBase::Main {
                review_base_oid: old_head_oid.clone(),
            },
            main_ancestor_oid: old_head_oid.clone(),
            rewrote_history: false,
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
        stack_status: scope_domain::requests::RequestStackStatus::Main,
        stack_depth: 1,
        id: "request".into(),
        repo_id: "owner/repo".into(),
        name: "topic".into(),
        author_user_id: Some("owner".into()),
        author_role: RequestActorRole::Owner,
        view: ViewId::private(),
        base: scope_domain::requests::RequestBase::Main {
            review_base_oid: "a".repeat(40),
        },
        base_version: 1,
        main_ancestor_oid: "a".repeat(40),
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
    async fn put(&self, key: &str, bytes: Vec<u8>) -> Result<(), scope_storage::ObjectStoreError> {
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
async fn legacy_revision_restores_once_for_concurrent_reads_and_jobs_are_removed_on_error() {
    let root = tempfile::tempdir().unwrap();
    let (revision, bytes) = fixture(root.path());
    let mut state = AppState::test_state();
    state
        .object_store
        .put(&scope_storage::object_key(&revision.git_snapshot), bytes)
        .await
        .unwrap();
    let loads = Arc::new(AtomicUsize::new(0));
    state.object_store = Arc::new(CountingStore {
        inner: state.object_store.clone(),
        loads: loads.clone(),
    });
    let base = super::super::ensure_local_bare_repo(root.path().join("base.git")).unwrap();
    run_git(
        Some(&base),
        &[
            "fetch",
            root.path().join("source").to_str().unwrap(),
            &revision.old_head_oid,
        ],
        "prepare legacy base",
    )
    .unwrap();
    let shelf = super::super::ensure_local_bare_repo(root.path().join("shelf.git")).unwrap();
    add_receive_pack_alternate(&shelf, &base.join("objects")).unwrap();
    let mut broken = revision.clone();
    broken.git_snapshot.sha256 = "0".repeat(64);
    let failed_state = state.clone();
    let failed_shelf = shelf.clone();
    assert!(
        crate::git::blocking::run(move || restore_revision(
            &failed_state,
            &failed_shelf,
            "refs/heads/topic",
            &broken
        ))
        .await
        .is_err()
    );
    assert!(!super::super::request_ref_oid_is_commit(&shelf, &revision.new_head_oid).unwrap());
    loads.store(0, Ordering::SeqCst);
    let read = || {
        let state = state.clone();
        let revision = revision.clone();
        let shelf = shelf.clone();
        async move {
            let install_state = state.clone();
            let install_shelf = shelf.clone();
            let install_revision = revision.clone();
            state
                .repository_engine
                .install_request_objects(&shelf, revision_heads(&revision), move || async move {
                    crate::git::blocking::run(move || {
                        restore_revision(
                            &install_state,
                            &install_shelf,
                            "refs/heads/topic",
                            &install_revision,
                        )
                    })
                    .await
                })
                .await
                .unwrap();
            inspect_revision(
                &state,
                &shelf,
                &request_fixture(),
                &revision,
                None,
                move |repo, revision| {
                    let result = run_git_output(
                        Some(repo),
                        &[
                            "diff",
                            &revision.old_head_oid,
                            &revision.new_head_oid,
                            "--",
                            "file.txt",
                        ],
                        "inspect legacy revision",
                    )?;
                    Ok((repo.to_path_buf(), result))
                },
            )
            .await
            .unwrap()
        }
    };
    let (first, second) = tokio::join!(read(), read());
    for (job, result) in [first, second, read().await] {
        assert!(!job.exists());
        assert!(
            String::from_utf8(result.stdout)
                .unwrap()
                .contains("+changed")
        );
    }
    let error_path = Arc::new(std::sync::Mutex::new(None));
    let captured = error_path.clone();
    let failed: Result<(), ApiError> = inspect_revision(
        &state,
        &shelf,
        &request_fixture(),
        &revision,
        None,
        move |repo, _| {
            *captured.lock().unwrap() = Some(repo.to_path_buf());
            Err(ApiError::bad_request("inspection failed"))
        },
    )
    .await;
    assert!(failed.is_err());
    assert!(!error_path.lock().unwrap().as_ref().unwrap().exists());
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    assert!(
        fs::read_dir(state.repository_engine.cache_root())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("revision-"))
    );
    eprintln!(
        "revision proof: two concurrent cold reads and a warm read, one legacy bundle load, zero retained revision repositories or jobs"
    );
}
