use super::*;
use crate::db::{MetadataStore, TestDatabaseTarget, generated_ids::test_generated_id};
use scope_domain::{
    account::UserAccount,
    dependency_analysis::{DependencyCheckStatus, DependencyEdge, DependencyFinding},
    policy::Visibility,
    repo_config::{ConfigVisibility, RepoConfigVisibilityRule},
    repository::{RepoLifecycleState, Repository, git::GitHead},
};

const NOW: u64 = 1_700_000_000;
const REPO_ID: &str = "dependency-owner/repo";

fn output() -> AnalyzerOutput {
    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
        analyzed_files: vec!["public.ts".into(), "private.ts".into()],
        unsupported_files: Vec::new(),
        edges: vec![DependencyEdge {
            source_path: "public.ts".into(),
            target_path: "private.ts".into(),
            kind: "import".into(),
        }],
        gaps: Vec::new(),
    }
}

fn repository() -> Repository {
    let owner = UserAccount {
        id: "dependency-owner-id".into(),
        handle: "dependency-owner".into(),
        email: "dependency-owner@scope.test".into(),
        email_verified: true,
    };
    let mut repository =
        Repository::new(&owner, "repo", Visibility::Public, "repoi_dependency_test").unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    repository.git_head = Some(GitHead::new("a".repeat(40), 1, 1));
    repository
        .repo_config
        .visibility
        .rules
        .push(RepoConfigVisibilityRule {
            path: "/private.ts".into(),
            visibility: ConfigVisibility::Private,
        });
    repository
}

async fn seeded_store() -> MetadataStore {
    let store =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    store
        .repositories()
        .replace_repository_for_tests(repository())
        .await
        .unwrap();
    store
}

async fn schedule_and_claim(
    store: &MetadataStore,
    worker: &str,
    now_unix: u64,
    lease_seconds: u64,
) -> DependencyAnalysisClaim {
    store
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, now_unix, 10)
        .await
        .unwrap();
    store
        .jobs()
        .claim_dependency_analysis(
            worker,
            DEPENDENCY_ANALYZER_VERSION,
            now_unix,
            lease_seconds,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn retained_edges_are_reevaluated_and_stale_policy_completion_is_rejected() {
    let store = seeded_store().await;
    store
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW, 10)
        .await
        .unwrap();
    let initial = store
        .jobs()
        .claim_dependency_analysis(
            "worker-a",
            DEPENDENCY_ANALYZER_VERSION,
            NOW,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(initial.reusable_analysis.is_none());
    assert_eq!(
        store
            .jobs()
            .complete_dependency_analysis_claim(&initial, output(), NOW + 1)
            .await
            .unwrap(),
        DependencyCompletion::Completed
    );
    let current = store
        .repositories()
        .dependency_check("dependency-owner", "repo", "dependency-owner-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current.status, DependencyCheckStatus::Ready);
    assert_eq!(
        current.report.unwrap().findings,
        vec![DependencyFinding {
            source_path: "public.ts".into(),
            target_path: "private.ts".into(),
        }]
    );

    store
        .repositories()
        .mutate_repository_for_tests(REPO_ID, |repository| {
            repository.repo_config.visibility.rules.clear();
            repository.bump_change_version();
        })
        .await
        .unwrap();
    let old_policy = store
        .jobs()
        .claim_dependency_analysis(
            "worker-a",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 2,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(old_policy.reusable_analysis.is_some());

    store
        .repositories()
        .mutate_repository_for_tests(REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    assert_eq!(
        store
            .jobs()
            .complete_reused_dependency_analysis_claim(&old_policy, NOW + 3)
            .await
            .unwrap(),
        DependencyCompletion::Stale
    );
    let updating = store
        .repositories()
        .dependency_check("dependency-owner", "repo", "dependency-owner-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updating.status, DependencyCheckStatus::Updating);
    assert_eq!(updating.report.unwrap().findings.len(), 1);

    let current_policy = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 3,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(current_policy.reusable_analysis.is_some());
    assert_eq!(
        store
            .jobs()
            .complete_reused_dependency_analysis_claim(&current_policy, NOW + 4)
            .await
            .unwrap(),
        DependencyCompletion::Completed
    );
    let refreshed = store
        .repositories()
        .dependency_check("dependency-owner", "repo", "dependency-owner-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refreshed.status, DependencyCheckStatus::Ready);
    assert!(refreshed.report.unwrap().findings.is_empty());
}

#[tokio::test]
async fn failed_refresh_retains_the_previous_report() {
    let store = seeded_store().await;
    store
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW, 10)
        .await
        .unwrap();
    let initial = store
        .jobs()
        .claim_dependency_analysis(
            "worker-a",
            DEPENDENCY_ANALYZER_VERSION,
            NOW,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    store
        .jobs()
        .complete_dependency_analysis_claim(&initial, output(), NOW + 1)
        .await
        .unwrap();
    store
        .repositories()
        .mutate_repository_for_tests(REPO_ID, Repository::bump_change_version)
        .await
        .unwrap();
    let refresh = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 2,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .jobs()
            .fail_dependency_analysis_claim(&refresh, "reader failed", NOW + 3)
            .await
            .unwrap()
    );

    let failed = store
        .repositories()
        .dependency_check("dependency-owner", "repo", "dependency-owner-id")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.status, DependencyCheckStatus::Failed);
    assert_eq!(failed.error.as_deref(), Some("reader failed"));
    assert_eq!(failed.report.unwrap().findings.len(), 1);
}

#[tokio::test]
async fn expired_completion_is_rejected_and_failure_uses_backoff() {
    let store = seeded_store().await;
    let expired = schedule_and_claim(&store, "worker-a", NOW, 5).await;
    assert_eq!(
        store
            .jobs()
            .complete_dependency_analysis_claim(&expired, output(), NOW + 5)
            .await
            .unwrap(),
        DependencyCompletion::Stale
    );

    let reclaimed = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 5,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(
        store
            .jobs()
            .fail_dependency_analysis_claim(&reclaimed, "reader failed", NOW + 6)
            .await
            .unwrap()
    );
    assert!(
        store
            .jobs()
            .claim_dependency_analysis(
                "worker-c",
                DEPENDENCY_ANALYZER_VERSION,
                NOW + 10,
                30,
                &test_generated_id,
            )
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        store
            .jobs()
            .claim_dependency_analysis(
                "worker-c",
                DEPENDENCY_ANALYZER_VERSION,
                NOW + 11,
                30,
                &test_generated_id,
            )
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn repository_recreation_fences_the_old_incarnation() {
    let store = seeded_store().await;
    let old = schedule_and_claim(&store, "worker-a", NOW, 30).await;
    let mut recreated = repository();
    recreated.record.incarnation_id = "repoi_dependency_recreated".into();
    recreated.git_head = Some(GitHead::new("b".repeat(40), 1, 1));
    store
        .repositories()
        .recreate_repository_for_tests(recreated)
        .await
        .unwrap();
    store
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW + 1, 10)
        .await
        .unwrap();

    assert_eq!(
        store
            .jobs()
            .complete_dependency_analysis_claim(&old, output(), NOW + 2)
            .await
            .unwrap(),
        DependencyCompletion::Stale
    );
    let current = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 2,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        current.incarnation.incarnation_id(),
        "repoi_dependency_recreated"
    );
    assert_eq!(current.git_head.head_oid, "b".repeat(40));
}

#[tokio::test]
async fn repeated_backfill_preserves_a_lease_and_repository_updates_enqueue_new_work() {
    let store = seeded_store().await;
    let claim = schedule_and_claim(&store, "worker-a", NOW, 30).await;
    store
        .jobs()
        .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW + 1, 10)
        .await
        .unwrap();
    assert!(
        store
            .jobs()
            .renew_dependency_analysis_claim(&claim, NOW + 2, 30)
            .await
            .unwrap()
    );
    store
        .jobs()
        .complete_dependency_analysis_claim(&claim, output(), NOW + 3)
        .await
        .unwrap();
    assert_eq!(
        store
            .jobs()
            .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW + 4, 10)
            .await
            .unwrap(),
        0
    );

    store
        .repositories()
        .mutate_repository_for_tests(REPO_ID, |repository| {
            repository.bump_change_version();
            repository.git_head = Some(GitHead::new("c".repeat(40), 2, 2));
        })
        .await
        .unwrap();
    let content_update = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 5,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(content_update.repo_version, 2);
    assert_eq!(content_update.git_head.head_oid, "c".repeat(40));
    assert!(content_update.reusable_analysis.is_none());
}

#[tokio::test]
async fn failed_first_page_job_does_not_starve_later_backfill_candidates() {
    let store = seeded_store().await;
    let first = schedule_and_claim(&store, "worker-a", NOW, 30).await;
    assert!(
        store
            .jobs()
            .fail_dependency_analysis_claim(&first, "reader failed", NOW + 1)
            .await
            .unwrap()
    );

    let mut second = repository();
    second.record.id = "dependency-owner/second".into();
    second.record.name = "second".into();
    second.record.incarnation_id = "repoi_dependency_second".into();
    second.graph.repo_id = second.record.id.clone();
    store
        .repositories()
        .replace_repository_for_tests(second)
        .await
        .unwrap();
    assert_eq!(
        store
            .jobs()
            .enqueue_dependency_analysis_backfill(DEPENDENCY_ANALYZER_VERSION, NOW + 2, 1,)
            .await
            .unwrap(),
        1
    );
    let second = store
        .jobs()
        .claim_dependency_analysis(
            "worker-b",
            DEPENDENCY_ANALYZER_VERSION,
            NOW + 2,
            30,
            &test_generated_id,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second.incarnation.repository_id(),
        "dependency-owner/second"
    );
}
