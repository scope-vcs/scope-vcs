use super::*;
use crate::git::{
    pool_pack::stage_pool_pack,
    test_repo::{TestRepo, pinned_identity_git},
};
use scope_domain::{
    account::UserAccount,
    repository::git::{GitHead, GitPoolPackOwner, RequestPackScope},
    requests::{RequestActorRole, RequestBase, RequestStackStatus},
    views::ViewId,
};
use std::collections::BTreeMap;

struct Fixture {
    state: AppState,
    repo: Repository,
    request: Request,
    revision: RequestRevision,
    source: TestRepo,
    bundle: std::path::PathBuf,
}

impl Fixture {
    async fn new(view: ViewId) -> Self {
        let state = AppState::test_state();
        let source = TestRepo::init();
        fs::write(source.join("file"), "old projected base").unwrap();
        source.commit_all("old projected base");
        let base = source.oid("HEAD");
        fs::write(source.join("file"), "request change").unwrap();
        if view.is_private() {
            let mut random = 932_u32;
            let contents: Vec<u8> = (0..1_100_000)
                .map(|_| {
                    random ^= random << 13;
                    random ^= random >> 17;
                    random ^= random << 5;
                    random as u8
                })
                .collect();
            fs::write(source.join("archive.bin"), contents).unwrap();
        }
        source.commit_all("request change");
        let head = source.oid("HEAD");
        pinned_identity_git(&source, &["branch", "topic"], None);
        pinned_identity_git(&source, &["checkout", "--orphan", "current-main"], None);
        fs::write(source.join("file"), "unrelated current main").unwrap();
        source.commit_all("current main");
        let main = source.oid("HEAD");
        let bundle = source.join("recovery.bundle");
        run_git(
            Some(&source),
            &["bundle", "create", bundle.to_str().unwrap(), "--all"],
            "create recovery fixture",
        )
        .unwrap();
        let owner = UserAccount {
            id: "owner".into(),
            handle: "owner".into(),
            email: "owner@example.invalid".into(),
            email_verified: true,
        };
        let mut repo =
            Repository::new(&owner, "repo", ViewId::private(), "repoi_recovery").unwrap();
        repo.git_head = Some(GitHead::new(main.clone(), 1, 1));
        let request = Request {
            id: "request".into(),
            repo_id: repo.record.id.clone(),
            name: "topic".into(),
            author_user_id: Some(owner.id.clone()),
            author_role: RequestActorRole::Owner,
            view,
            base: RequestBase::Main {
                review_base_oid: base.clone(),
            },
            base_version: 1,
            stack_status: RequestStackStatus::Main,
            stack_depth: 1,
            main_ancestor_oid: base.clone(),
            head_oid: head.clone(),
            pushed_at_unix: Some(1),
            title: "recovery".into(),
            description_markdown: String::new(),
            activity_version: 2,
            submitted_at_unix: Some(1),
            closed_at_unix: None,
            closed_by_user_id: None,
            merged_at_unix: Some(2),
            merged_by_user_id: Some(owner.id.clone()),
            merged_head_oid: Some(head.clone()),
            merged_main_oid: Some(main.clone()),
            created_at_unix: 1,
            updated_at_unix: 2,
        };
        let revision = RequestRevision {
            id: "revision".into(),
            request_id: request.id.clone(),
            position: 2,
            actor_user_id: Some(owner.id.clone()),
            old_head_oid: base.clone(),
            new_head_oid: head,
            base: request.base.clone(),
            base_version: 1,
            main_ancestor_oid: base,
            rewrote_history: false,
            created_at_unix: 1,
        };
        state
            .metadata
            .admin()
            .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
                users: BTreeMap::from([(owner.id.clone(), owner)]),
                repositories: BTreeMap::from([(repo.record.id.clone(), repo.clone())]),
                requests: BTreeMap::from([(request.id.clone(), request.clone())]),
                request_revisions: BTreeMap::from([(revision.id.clone(), revision.clone())]),
                ..Default::default()
            })
            .unwrap();
        let pack = stage_pool_pack(
            &state,
            &repo.record.id,
            &source,
            PoolPackRevision {
                owner: GitPoolPackOwner::Main {
                    first_sequence: 1,
                    last_sequence: 1,
                },
                head_oid: main,
                prerequisites: Vec::new(),
            },
        )
        .await
        .unwrap();
        state
            .metadata
            .repositories()
            .register_main_pool_pack_for_tests(&repo.record.id, &pack.pack)
            .await
            .unwrap();
        pack.finish(&state).await;
        Self {
            state,
            repo,
            request,
            revision,
            source,
            bundle,
        }
    }

    async fn packs(&self) -> Vec<scope_postgres::db::PooledRequestPack> {
        self.state
            .metadata
            .repositories()
            .git_pool_packs_for_requests(
                &self.repo.record.id,
                std::slice::from_ref(&self.request.id),
            )
            .await
            .unwrap()
    }

    async fn restore(&self, write: bool) -> Result<Vec<RestoreOutcome>, ApiError> {
        restore(
            &self.state,
            &self.repo.record.id,
            &self.bundle,
            std::slice::from_ref(&self.revision.id),
            write,
        )
        .await
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restore_repairs_missing_prerequisite_from_an_empty_shelf_without_duplicate_writes() {
    let fixture = Fixture::new(ViewId::private()).await;
    let broken = stage_pool_pack(
        &fixture.state,
        &fixture.repo.record.id,
        &fixture.source,
        PoolPackRevision {
            owner: GitPoolPackOwner::Request {
                request_id: fixture.request.id.clone(),
                scope: RequestPackScope::Full,
            },
            head_oid: fixture.revision.new_head_oid.clone(),
            prerequisites: vec![fixture.revision.old_head_oid.clone()],
        },
    )
    .await
    .unwrap();
    fixture
        .state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(&fixture.repo.record.id, &broken.pack)
        .await
        .unwrap();
    broken.finish(&fixture.state).await;
    let before = read_revision(
        &fixture.state,
        &fixture.repo,
        &fixture.request,
        &fixture.revision,
    )
    .await
    .unwrap_err();
    assert!(
        before
            .into_operator_diagnostic()
            .contains("missing comparison commits")
    );
    println!("before restore: real revision reader failed: missing comparison commits");
    assert_eq!(
        fixture.restore(false).await.unwrap(),
        [RestoreOutcome::WouldRestore]
    );
    assert_eq!(fixture.packs().await.len(), 1);
    assert_eq!(
        fixture.restore(true).await.unwrap(),
        [RestoreOutcome::Restored]
    );
    let packs = fixture.packs().await;
    assert_eq!(packs.len(), 2);
    let repaired = packs
        .iter()
        .find(|pack| pack.prerequisites.is_empty())
        .unwrap();
    let storage = fixture
        .state
        .metadata
        .repositories()
        .git_pool_pack_storage(&fixture.repo.record.id, std::slice::from_ref(&repaired.id))
        .await
        .unwrap();
    assert!(matches!(
        storage[0].1,
        scope_domain::repository::git::GitPoolPackStorage::Segment(_)
    ));
    read_revision(
        &fixture.state,
        &fixture.repo,
        &fixture.request,
        &fixture.revision,
    )
    .await
    .unwrap();
    println!("after restore: real revision reader verified from an empty shelf");
    assert_eq!(
        fixture.restore(true).await.unwrap(),
        [RestoreOutcome::AlreadyReadable]
    );
    assert_eq!(fixture.packs().await.len(), packs.len());
    let sweep = sweep(&fixture.state, Some(&fixture.repo.record.id))
        .await
        .unwrap();
    assert_eq!(sweep.scanned, 1);
    assert!(sweep.failures.is_empty());
    let mut changed = fixture.request.clone();
    changed.activity_version += 1;
    let candidate = stage_pool_pack_revisions(
        &fixture.state,
        &fixture.repo.record.id,
        &fixture.source,
        PoolPackRevision {
            owner: fixture
                .revision
                .restoration_pack_owner(&fixture.request, fixture.repo.repo_config.views())
                .unwrap(),
            head_oid: fixture.revision.new_head_oid.clone(),
            prerequisites: Vec::new(),
        },
        fixture.revision.comparison_oids(),
        Some(recovery_pack_id(&fixture.repo, &fixture.revision).unwrap()),
    )
    .await
    .unwrap();
    assert!(
        !fixture
            .state
            .metadata
            .repositories()
            .restore_revision_pool_pack(
                &fixture.repo,
                &fixture.request,
                &fixture.revision,
                &candidate.pack,
                3
            )
            .await
            .unwrap()
    );
    assert!(
        fixture
            .state
            .metadata
            .repositories()
            .restore_revision_pool_pack(
                &fixture.repo,
                &changed,
                &fixture.revision,
                &candidate.pack,
                3
            )
            .await
            .is_err()
    );
    candidate
        .discard(&fixture.state, &fixture.repo.record.id)
        .await;
    fixture
        .state
        .metadata
        .requests()
        .mutate_request_for_tests(&fixture.request.id, |request| {
            request.merged_at_unix = None;
            request.merged_by_user_id = None;
            request.merged_head_oid = None;
            request.merged_main_oid = None;
        })
        .await
        .unwrap();
    assert!(fixture.restore(true).await.is_err());
    assert_eq!(fixture.packs().await.len(), packs.len());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sweep_checks_review_base_and_main_ancestor_and_restore_includes_disconnected_comparisons()
{
    let mut fixture = Fixture::new(ViewId::public()).await;
    let mut revisions = Vec::new();
    for (branch, missing_review_base) in [("review-base", true), ("main-ancestor", false)] {
        pinned_identity_git(&fixture.source, &["checkout", "--orphan", branch], None);
        fs::write(fixture.source.join("file"), branch).unwrap();
        fixture.source.commit_all(branch);
        let missing = fixture.source.oid("HEAD");
        let mut revision = fixture.revision.clone();
        revision.id = branch.into();
        revision.position = if missing_review_base { 3 } else { 4 };
        if missing_review_base {
            revision.base = RequestBase::Main {
                review_base_oid: missing,
            };
        } else {
            revision.main_ancestor_oid = missing;
        }
        revisions.push(revision);
    }
    fixture
        .state
        .metadata
        .admin()
        .seed_catalog_for_tests(scope_postgres::db::CatalogFixture {
            request_revisions: revisions
                .iter()
                .cloned()
                .map(|revision| (revision.id.clone(), revision))
                .collect(),
            ..Default::default()
        })
        .unwrap();
    let head_only = stage_pool_pack(
        &fixture.state,
        &fixture.repo.record.id,
        &fixture.source,
        PoolPackRevision {
            owner: fixture
                .revision
                .restoration_pack_owner(&fixture.request, fixture.repo.repo_config.views())
                .unwrap(),
            head_oid: fixture.revision.new_head_oid.clone(),
            prerequisites: Vec::new(),
        },
    )
    .await
    .unwrap();
    fixture
        .state
        .metadata
        .repositories()
        .register_main_pool_pack_for_tests(&fixture.repo.record.id, &head_only.pack)
        .await
        .unwrap();
    head_only.finish(&fixture.state).await;
    let report = sweep(&fixture.state, None).await.unwrap();
    assert_eq!(report.scanned, 3);
    assert_eq!(
        report
            .failures
            .iter()
            .map(|failure| failure.revision.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["review-base", "main-ancestor"])
    );
    assert!(
        report
            .failures
            .iter()
            .all(|failure| failure.error.contains("missing comparison commits"))
    );
    println!("sweep: detected unreadable review-base and main-ancestor revisions");
    fs::remove_file(&fixture.bundle).unwrap();
    run_git(
        Some(&fixture.source),
        &[
            "bundle",
            "create",
            fixture.bundle.to_str().unwrap(),
            "--all",
        ],
        "update fixture bundle",
    )
    .unwrap();
    fixture.revision = revisions.remove(0);
    assert_eq!(
        fixture.restore(true).await.unwrap(),
        [RestoreOutcome::Restored]
    );
    let report = sweep(&fixture.state, Some(&fixture.repo.record.id))
        .await
        .unwrap();
    assert_eq!(report.failures.len(), 1);
    assert_eq!(report.failures[0].revision, "main-ancestor");
    fixture.revision = revisions.remove(0);
    assert_eq!(
        fixture.restore(true).await.unwrap(),
        [RestoreOutcome::Restored]
    );
    assert!(
        sweep(&fixture.state, None)
            .await
            .unwrap()
            .failures
            .is_empty()
    );
}

#[test]
fn recovery_arguments_require_explicit_write_and_reject_ambiguous_selection() {
    let parse = |arguments: &[&str]| {
        Command::parse(
            &arguments
                .iter()
                .map(|arg| arg.to_string())
                .collect::<Vec<_>>(),
        )
    };
    assert!(matches!(
        parse(&["restore", "owner/repo", "heads.bundle", "revision"]).unwrap(),
        Command::Restore { write: false, .. }
    ));
    assert!(matches!(
        parse(&[
            "restore",
            "owner/repo",
            "heads.bundle",
            "revision",
            "--write"
        ])
        .unwrap(),
        Command::Restore { write: true, .. }
    ));
    for arguments in [
        vec!["restore", "owner/repo", "heads.bundle", "--write"],
        vec![
            "restore",
            "owner/repo",
            "heads.bundle",
            "revision",
            "revision",
        ],
        vec!["sweep", "--write"],
    ] {
        assert!(parse(&arguments).is_err());
    }
}
