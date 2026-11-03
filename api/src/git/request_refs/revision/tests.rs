use super::*;
use crate::git::{command::run_git_output, import::git_snapshot_from_ref};
use scope_domain::requests::RequestActorRole;
use scope_domain::views::ViewId;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn fixture() -> (crate::git::test_repo::TestRepo, RequestRevision, Vec<u8>) {
    let source = crate::git::test_repo::TestRepo::init();
    crate::git::test_repo::pinned_identity_git(&source, &["branch", "-m", "topic"], None);
    fs::write(source.join("file.txt"), "base\n").unwrap();
    source.commit_all("fixture");
    let old_head_oid = source.oid("HEAD");
    fs::write(source.join("file.txt"), "changed\n").unwrap();
    source.commit_all("fixture");
    let new_head_oid = source.oid("HEAD");
    let (git_snapshot, bytes) =
        git_snapshot_from_ref(&source, "refs/heads/topic", Some(&old_head_oid)).unwrap();
    (
        source,
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

fn seed_repository(
    state: &AppState,
    revisions: Vec<RequestRevision>,
) -> (RepositoryIncarnation, String) {
    let owner = scope_domain::account::UserAccount {
        id: "owner".into(),
        handle: "owner".into(),
        email: "owner@example.invalid".into(),
        email_verified: true,
    };
    let repo = scope_domain::repository::Repository::new(
        &owner,
        "repo",
        ViewId::private(),
        "repoi_revision",
    )
    .unwrap();
    let incarnation = repo.record.incarnation();
    let generation = repo.repo_config.object_generation();
    state
        .metadata
        .admin()
        .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
            users: std::collections::BTreeMap::from([(owner.id.clone(), owner)]),
            repositories: std::collections::BTreeMap::from([(repo.record.id.clone(), repo)]),
            requests: std::collections::BTreeMap::from([("request".into(), request_fixture())]),
            request_revisions: revisions
                .into_iter()
                .map(|revision| (revision.id.clone(), revision))
                .collect(),
            ..Default::default()
        })
        .unwrap();
    (incarnation, generation)
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
    let (source, revision, bytes) = fixture();
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
    let (incarnation, generation) = seed_repository(&state, Vec::new());
    let shelf = request_object_repos(&state, &incarnation, &generation, "owner")
        .unwrap()
        .shared;
    run_git(
        Some(&shelf),
        &["fetch", source.to_str().unwrap(), &revision.old_head_oid],
        "prepare legacy base",
    )
    .unwrap();
    let mut broken = revision.clone();
    broken.git_snapshot.sha256 = "0".repeat(64);
    assert!(
        with_request_revision_store_repo(
            &state,
            &incarnation,
            &request_fixture(),
            &broken,
            |_, _| Ok(())
        )
        .await
        .is_err()
    );
    assert!(!super::super::request_ref_oid_is_commit(&shelf, &revision.new_head_oid).unwrap());
    loads.store(0, Ordering::SeqCst);
    let read = || {
        let state = state.clone();
        let revision = revision.clone();
        let incarnation = incarnation.clone();
        async move {
            with_request_revision_store_repo(
                &state,
                &incarnation,
                &request_fixture(),
                &revision,
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
    state
        .metadata
        .repositories()
        .mutate_repository_for_tests("owner/repo", |repo| {
            repo.bump_content_version();
            repo.git_head = Some(scope_domain::repository::git::GitHead::new(
                "f".repeat(40),
                1,
                1,
            ));
        })
        .await
        .unwrap();
    let canonical = state.repository_engine.repository_path(&incarnation);
    super::super::ensure_local_bare_repo(canonical.clone()).unwrap();
    let canonical_for_read = canonical.clone();
    let engine = state.repository_engine.clone();
    let incarnation_for_read = incarnation.clone();
    with_request_revision_store_repo(
        &state,
        &incarnation,
        &request_fixture(),
        &revision,
        move |_, _| {
            if engine
                .delete_repository_cache(&incarnation_for_read)
                .unwrap()
                || !canonical_for_read.exists()
            {
                return Err(ApiError::internal_message(
                    "canonical lease was lost during the read",
                ));
            }
            Ok(())
        },
    )
    .await
    .unwrap();
    let failed: Result<(), ApiError> = with_request_revision_store_repo(
        &state,
        &RepositoryIncarnation::new("owner/repo", "repoi_revision").unwrap(),
        &request_fixture(),
        &revision,
        move |repo, _| {
            *captured.lock().unwrap() = Some(repo.to_path_buf());
            Err(ApiError::bad_request("inspection failed"))
        },
    )
    .await;
    assert!(failed.is_err());
    assert!(!error_path.lock().unwrap().as_ref().unwrap().exists());
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

#[test]
fn startup_removes_abandoned_revision_jobs() {
    let root = tempfile::tempdir().unwrap();
    let abandoned = root.path().join("git-jobs/revision-abandoned");
    fs::create_dir_all(&abandoned).unwrap();
    fs::write(abandoned.join("objects"), "abandoned").unwrap();
    sweep_revision_jobs(root.path()).unwrap();
    assert!(!abandoned.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn full_view_revision_job_cannot_borrow_authored_shelf_objects() {
    let (source, mut revision, _) = fixture();
    let hidden = revision.new_head_oid.clone();
    let state = AppState::test_state();
    let (incarnation, generation) = seed_repository(&state, Vec::new());
    let repos = request_object_repos(&state, &incarnation, &generation, "owner").unwrap();
    for (repo, head) in [
        (&repos.shared, &revision.old_head_oid),
        (&repos.authored, &hidden),
    ] {
        run_git(
            Some(repo),
            &["fetch", source.to_str().unwrap(), head],
            "prepare shelf isolation",
        )
        .unwrap();
    }
    revision.new_head_oid = revision.old_head_oid.clone();
    let (readable, leaked) = with_request_revision_store_repo(
        &state,
        &incarnation,
        &request_fixture(),
        &revision,
        move |job, revision| {
            let present = present_commits(
                job,
                [&revision.new_head_oid, &hidden]
                    .into_iter()
                    .map(String::as_str),
            )?;
            Ok((
                present.contains(&revision.new_head_oid),
                present.contains(&hidden),
            ))
        },
    )
    .await
    .unwrap();
    assert!(readable);
    assert!(!leaked, "full-view shared job reached an author's draft");
}
