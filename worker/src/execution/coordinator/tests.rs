use super::*;
use crate::execution::fake::{FakeEcs, TEST_IMAGE};
use axum::http::StatusCode;
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    content_ref::ContentRef,
    policy::Visibility,
    repository::{RepoLifecycleState, Repository},
    runs::{
        run::Run,
        source::{RunSource, RunTrigger},
        workflow::{
            definition::{
                CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
                WorkflowTriggers,
            },
            identity::{WorkflowIdentity, WorkflowPath},
            revision::WorkflowRevision,
        },
    },
};
use scope_postgres::db::{CatalogFixture, TestDatabaseTarget};
use serde_json::json;

#[tokio::test]
async fn zero_cloud_concurrency_pauses_admission() {
    let metadata = queued_runs(1).await;
    let provider = FakeEcs::new().await;
    let mut settings = provider.settings();
    settings.max_concurrency = 0;
    let coordinator = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        product_analytics: scope_product_analytics::ProductAnalytics::disabled(),
        ecs: provider.client.clone(),
        origin_id: "paused-worker".into(),
        settings,
    };
    assert_eq!(
        coordinator
            .dispatch_available(crate::unix_now().unwrap())
            .await
            .unwrap(),
        0
    );
    assert_eq!(provider.count("start"), 0);
    assert!(
        metadata
            .runs()
            .next_dispatchable_job()
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn paused_dispatch_still_checks_cleanup_without_marking_stopping_as_complete() {
    let metadata = queued_runs(2).await;
    let now = crate::unix_now().unwrap();
    let attempt_ids = ["attempt_paused_cleanup_1", "attempt_paused_cleanup_2"];
    let expired_at = now + DISPATCH_LEASE.as_secs() + 1;
    for (index, attempt_id) in attempt_ids.into_iter().enumerate() {
        let scope_postgres::db::DispatchAdmission::Admitted(_) = metadata
            .runs()
            .admit_next_job(
                2,
                attempt_id,
                &format!("{:064x}", index + 1),
                "test",
                now,
                now + DISPATCH_LEASE.as_secs(),
            )
            .await
            .unwrap()
        else {
            panic!("expected admitted attempt");
        };
        metadata
            .runs()
            .expire_attempt(attempt_id, expired_at)
            .await
            .unwrap();
    }
    let provider = FakeEcs::new().await;
    provider.reply(
        StatusCode::OK,
        json!({"status":"stopping", "stuck":false}),
        false,
    );
    let mut settings = provider.settings();
    settings.max_concurrency = 0;
    let coordinator = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        product_analytics: scope_product_analytics::ProductAnalytics::disabled(),
        ecs: provider.client.clone(),
        origin_id: "paused-worker".into(),
        settings,
    };
    assert_eq!(coordinator.cleanup_terminal(expired_at).await.unwrap(), 0);
    assert_eq!(provider.count("stop"), 2);
    let pending = metadata
        .runs()
        .claim_terminal_cloud_task_stops(expired_at + 1, 2)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending
            .iter()
            .all(|task| attempt_ids.contains(&task.attempt_id.as_str()))
    );
}

#[tokio::test]
async fn attempt_analytics_correlates_admission_and_completion_without_replaying_completion() {
    let metadata = queued_runs(1).await;
    let (analytics, recording) = scope_product_analytics::ProductAnalytics::recording_for_source(
        scope_product_analytics::EventSource::Worker,
    );
    let now = crate::unix_now().unwrap();
    let token_hash = "b".repeat(64);
    let scope_postgres::db::DispatchAdmission::Admitted(claim) = metadata
        .runs()
        .admit_next_job(
            1,
            "attempt_analytics",
            &token_hash,
            "test",
            now,
            now + DISPATCH_LEASE.as_secs(),
        )
        .await
        .unwrap()
    else {
        panic!("expected workflow attempt admission");
    };
    analytics.capture_workflow_attempt_started(
        claim.repository.incarnation_id(),
        &claim.run,
        &claim.attempt,
    );
    assert_eq!(recording.event_names(), ["workflow:attempt_start"]);

    let conclusion = || AttemptConclusion::SetupFailed {
        exit_code: 69,
        message: "provider rejected dispatch".into(),
    };
    let completed = metadata
        .runs()
        .complete_attempt(&claim.attempt.id, &token_hash, conclusion(), false, now + 2)
        .await
        .unwrap();
    assert!(completed.transitioned);
    capture_attempt_completed(&analytics, &completed);

    let replayed = metadata
        .runs()
        .complete_attempt(&claim.attempt.id, &token_hash, conclusion(), false, now + 3)
        .await
        .unwrap();
    assert!(!replayed.transitioned);
    capture_attempt_completed(&analytics, &replayed);

    assert_eq!(
        recording.event_names(),
        ["workflow:attempt_start", "workflow:attempt_complete"]
    );
    assert_eq!(
        recording.property(0, "repository_id"),
        Some(serde_json::Value::String("repoi_worker_test".into()))
    );
    assert_eq!(
        recording.property(0, "run_id"),
        recording.property(1, "run_id")
    );
    assert_eq!(
        recording.property(0, "attempt_id"),
        recording.property(1, "attempt_id")
    );
    assert_eq!(
        recording.property(1, "result"),
        Some(serde_json::Value::String("failed".into()))
    );
    assert_eq!(
        recording.property(1, "run_result"),
        Some(serde_json::Value::String("failed".into()))
    );
    assert_eq!(recording.property(1, "duration_ms"), Some(2_000.into()));
}

#[tokio::test]
async fn interrupted_provider_starts_and_cleanup_remain_owned_after_worker_restart() {
    let metadata = queued_runs(3).await;
    let provider = FakeEcs::new().await;
    let coordinator = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        product_analytics: scope_product_analytics::ProductAnalytics::disabled(),
        ecs: provider.client.clone(),
        origin_id: "worker-before-restart".into(),
        settings: provider.settings(),
    };
    let now = crate::unix_now().unwrap();
    let mut dispatch = tokio::task::JoinSet::new();
    dispatch.spawn(async move { coordinator.dispatch_available(now).await });
    // Actual coordinator admissions commit before concurrent broker start calls.
    provider.wait_for("start", 3).await;
    let mut bootstrap_hashes = provider
        .bootstrap_tokens()
        .into_iter()
        .map(|token| {
            let secret = token
                .strip_prefix("scope_bootstrap_")
                .expect("dispatched credentials must use the runtime bootstrap prefix");
            assert_eq!(hex::decode(secret).unwrap().len(), 32);
            hex::encode(Sha256::digest(token.as_bytes()))
        })
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(
        bootstrap_hashes.len(),
        3,
        "each attempt gets its own credential"
    );
    assert_eq!(
        metadata
            .runs()
            .expired_attempt_ids(now + DISPATCH_LEASE.as_secs() + 1, 10)
            .await
            .unwrap()
            .len(),
        3
    );
    dispatch.shutdown().await;
    // AWS may finish those requests even though this worker lost the response.
    provider.starts.add_permits(3);
    let expired_at = now + DISPATCH_LEASE.as_secs() + 1;
    let expired = metadata
        .runs()
        .expired_attempt_ids(expired_at, 10)
        .await
        .unwrap();
    assert_eq!(expired.len(), 3);
    for attempt in &expired {
        let expired_claim = metadata
            .runs()
            .expire_attempt(attempt, expired_at)
            .await
            .unwrap()
            .claim;
        assert!(
            bootstrap_hashes.remove(&expired_claim.attempt.token_hash),
            "the dispatched credential hash must match an admitted attempt"
        );
        assert_eq!(
            expired_claim.job.state,
            scope_domain::runs::job::RunJobState::Queued
        );
    }
    assert!(
        matches!(
            metadata
                .runs()
                .admit_next_job(
                    4,
                    "attempt_retry_probe",
                    &"b".repeat(64),
                    "test",
                    expired_at,
                    expired_at + DISPATCH_LEASE.as_secs(),
                )
                .await
                .unwrap(),
            scope_postgres::db::DispatchAdmission::Empty
        ),
        "a retry cannot bypass cleanup of the previous uncertain provider task"
    );

    let restarted = CloudExecutionCoordinator {
        metadata: metadata.clone(),
        product_analytics: scope_product_analytics::ProductAnalytics::disabled(),
        ecs: provider.client.clone(),
        origin_id: "worker-after-restart".into(),
        settings: provider.settings(),
    };
    let mut cleanup = tokio::task::JoinSet::new();
    cleanup.spawn(async move { restarted.cleanup_terminal(expired_at).await });
    provider.wait_for("stop", 3).await;
    cleanup.shutdown().await;
    assert!(
        metadata
            .runs()
            .claim_terminal_cloud_task_stops(expired_at + 1, 10)
            .await
            .unwrap()
            .is_empty()
    );
    // A second restart cannot lose the unknown outcome or prematurely free its fence.
    let reclaimed = metadata
        .runs()
        .claim_terminal_cloud_task_stops(expired_at + 901, 10)
        .await
        .unwrap();
    assert_eq!(reclaimed.len(), 3);
    for task in reclaimed {
        assert!(expired.contains(&task.attempt_id));
        assert!(task.external_run_id.is_none());
    }
}

#[tokio::test]
async fn competing_workers_reserve_capacity_before_concurrent_provider_starts() {
    let metadata = queued_runs(12).await;
    let provider = FakeEcs::new().await;
    let now = crate::unix_now().unwrap();
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(3));
    let mut workers = tokio::task::JoinSet::new();
    for origin in ["worker-a", "worker-b"] {
        let mut settings = provider.settings();
        settings.max_concurrency = 3;
        let coordinator = CloudExecutionCoordinator {
            metadata: metadata.clone(),
            product_analytics: scope_product_analytics::ProductAnalytics::disabled(),
            ecs: provider.client.clone(),
            origin_id: origin.into(),
            settings,
        };
        let barrier = barrier.clone();
        workers.spawn(async move {
            barrier.wait().await;
            coordinator.dispatch_available(now).await
        });
    }
    barrier.wait().await;
    provider.wait_for("start", 3).await;
    assert!(
        matches!(
            metadata
                .runs()
                .admit_next_job(
                    3,
                    "attempt_capacity_probe",
                    &"b".repeat(64),
                    "test",
                    now,
                    now + DISPATCH_LEASE.as_secs(),
                )
                .await
                .unwrap(),
            scope_postgres::db::DispatchAdmission::AtCapacity
        ),
        "held provider requests already own all three database capacity reservations"
    );
    // Let every unexpected start finish as well, so an over-admission fails the count.
    provider.starts.add_permits(12);
    let dispatched = tokio::time::timeout(Duration::from_secs(5), async {
        let mut dispatched = 0;
        while let Some(result) = workers.join_next().await {
            dispatched += result.unwrap().unwrap();
        }
        dispatched
    })
    .await
    .expect("both coordinator ticks finish after provider responses");
    assert_eq!(dispatched, 3);
    assert_eq!(provider.count("start"), 3);
    assert_eq!(provider.peak_starts(), 3);
    let mut admitted_runs = 0;
    for id in 0..12 {
        let run = metadata
            .runs()
            .run(&format!("run-{id}"))
            .await
            .unwrap()
            .unwrap();
        match run.state {
            scope_domain::runs::run::RunState::Dispatching => admitted_runs += 1,
            scope_domain::runs::run::RunState::Queued => {}
            state => panic!("unexpected run state: {state:?}"),
        }
    }
    assert_eq!(admitted_runs, 3);
}

pub(super) async fn queued_runs(count: usize) -> MetadataStore {
    let metadata =
        MetadataStore::connect_fresh_for_tests(&TestDatabaseTarget::required().unwrap()).unwrap();
    let owner = UserAccount {
        id: "scope_usr_worker_owner".into(),
        handle: "worker-owner".into(),
        email: "owner@example.test".into(),
        email_verified: true,
    };
    let mut repository = Repository::new(
        &owner,
        "worker-test",
        Visibility::Private,
        "repoi_worker_test",
    )
    .unwrap();
    repository.record.lifecycle_state = RepoLifecycleState::Ready;
    let repo_id = repository.record.id.clone();
    let mut catalog = CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog.repositories.insert(repo_id.clone(), repository);
    metadata.admin().seed_catalog_for_tests(catalog).unwrap();
    let identity = WorkflowIdentity::new(
        repo_id,
        WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
    )
    .unwrap();
    let job = WorkflowJob::new(
        WorkflowJobId::parse("checks").unwrap(),
        vec![],
        ContainerSpec::new(TEST_IMAGE).unwrap(),
        600,
        vec![],
        Default::default(),
        vec![WorkflowStep::new("Test", "true").unwrap()],
    )
    .unwrap();
    let revision = WorkflowRevision::new(
        identity,
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false, false).unwrap(),
            vec![job],
        )
        .unwrap(),
    )
    .unwrap();
    let source = RunSource::ephemeral_git_bundle(SourceBlob {
        content_ref: ContentRef::git_bundle_sha256("c".repeat(64)),
        sha256: "c".repeat(64),
        git_oid: "d".repeat(40),
        git_file_mode: "100644".into(),
        size_bytes: 42,
    })
    .unwrap();
    for id in 0..count {
        let run = Run::new(
            format!("run-{id}"),
            format!("manual:{id}"),
            revision.workflow().clone(),
            revision.digest(),
            RunTrigger::Manual,
            Some(owner.id.clone()),
            source.clone(),
            crate::unix_now().unwrap(),
        )
        .unwrap();
        metadata
            .runs()
            .enqueue_run(run, revision.clone())
            .await
            .unwrap();
    }
    metadata
}
