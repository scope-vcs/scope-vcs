use crate::{
    db::{
        DispatchAdmission, MetadataStore, RecordRequestChecksCommand,
        requests::tests::postgres_store,
        test_support::fixtures::{repository, source_blob, store_with_repositories, user},
    },
    error::PostgresErrorKind,
};
use scope_domain::{
    policy::Visibility,
    requests::{
        NativeRequestCheck, RequestActorRole, RequestAudience, RequestCheck,
        RequestCheckEvaluation, RequestCheckEvaluationState, StartRequestInput,
    },
    runs::{
        availability::NATIVE_RUNS_UNAVAILABLE,
        run::{Run, RunState},
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

const OWNER_ID: &str = "user_native_owner";
const OWNER_HANDLE: &str = "native-owner";

fn workflow(repository_id: &str) -> WorkflowRevision {
    WorkflowRevision::new(
        WorkflowIdentity::new(
            repository_id,
            WorkflowPath::parse("/.scope/runs/test.yml").unwrap(),
        )
        .unwrap(),
        CompiledWorkflow::new(
            "Test",
            WorkflowTriggers::new(true, false, false).unwrap(),
            vec![
                WorkflowJob::new(
                    WorkflowJobId::parse("checks").unwrap(),
                    vec![],
                    ContainerSpec::new(format!("rust@sha256:{}", "a".repeat(64))).unwrap(),
                    600,
                    vec![],
                    Default::default(),
                    vec![WorkflowStep::new("Test", "cargo test").unwrap()],
                )
                .unwrap(),
            ],
        )
        .unwrap(),
    )
    .unwrap()
}

fn run(revision: &WorkflowRevision, id: &str) -> Run {
    Run::new(
        id,
        id,
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(OWNER_ID.into()),
        RunSource::ephemeral_git_bundle(source_blob(&"d".repeat(40), &"c".repeat(64), 42)).unwrap(),
        10,
    )
    .unwrap()
}

fn unlisted_store() -> (MetadataStore, WorkflowRevision) {
    let repo = repository(
        &user(OWNER_ID, OWNER_HANDLE),
        "native-repo",
        Visibility::Private,
    );
    let revision = workflow(&repo.record.id);
    (store_with_repositories([repo]), revision)
}

async fn admit(store: &MetadataStore) -> DispatchAdmission {
    store
        .runs()
        .admit_next_job(10, "attempt-1", &"e".repeat(64), "runtime", 11, 20)
        .await
        .unwrap()
}

#[tokio::test]
async fn operators_list_accounts_by_handle() {
    let (store, _) = unlisted_store();
    let native_runs = store.native_runs();
    assert_eq!(
        native_runs
            .add_account("missing", None, 1)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::NotFound
    );

    let added = native_runs
        .add_account(OWNER_HANDLE, Some(" design partner ".into()), 5)
        .await
        .unwrap();
    assert_eq!(added.listing.handle, OWNER_HANDLE);
    assert_eq!(added.listing.account.user_id, OWNER_ID);
    assert_eq!(
        added.listing.account.note.as_deref(),
        Some("design partner")
    );
    let owned = |repositories: &[scope_domain::repository::RepositoryIncarnation]| {
        repositories
            .iter()
            .map(|repository| repository.repository_id().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(owned(&added.repositories), ["native-owner/native-repo"]);
    let renoted = native_runs
        .add_account(OWNER_HANDLE, None, 9)
        .await
        .unwrap()
        .listing;
    assert_eq!(renoted.account.added_at_unix, 5);
    assert_eq!(renoted.account.note, None);
    assert_eq!(native_runs.accounts().await.unwrap(), [renoted]);

    // Every owned repository changes availability, even with nothing to settle.
    let removal = native_runs.remove_account(OWNER_HANDLE, 10).await.unwrap();
    assert!(removal.removed);
    assert_eq!(owned(&removal.repositories), ["native-owner/native-repo"]);
    assert!(native_runs.accounts().await.unwrap().is_empty());
    assert!(
        !native_runs
            .remove_account(OWNER_HANDLE, 11)
            .await
            .unwrap()
            .removed
    );
}

#[tokio::test]
async fn an_unlisted_owner_creates_no_runs_and_admits_none() {
    let (store, revision) = unlisted_store();
    let refused = store
        .runs()
        .enqueue_run(run(&revision, "run-refused"), revision.clone())
        .await
        .map(|_| ())
        .unwrap_err();
    assert_eq!(refused.kind, PostgresErrorKind::PermissionDenied);
    assert_eq!(refused.message, NATIVE_RUNS_UNAVAILABLE);
    assert!(
        !store
            .native_runs()
            .repository_availability(revision.workflow().repository_id())
            .await
            .unwrap()
            .is_available()
    );

    store
        .native_runs()
        .add_account(OWNER_HANDLE, None, 1)
        .await
        .unwrap();
    for id in ["run-1", "run-2"] {
        store
            .runs()
            .enqueue_run(run(&revision, id), revision.clone())
            .await
            .unwrap();
    }
    let DispatchAdmission::Admitted(claim) = admit(&store).await else {
        panic!("a listed owner's job is admitted");
    };
    assert_eq!(claim.run.id, "run-1");

    let removal = store
        .native_runs()
        .remove_account(OWNER_HANDLE, 12)
        .await
        .unwrap();
    assert!(matches!(admit(&store).await, DispatchAdmission::Empty));
    let canceled = removal
        .canceled_runs
        .iter()
        .map(|run| (run.id.as_str(), run.state, run.cancellation_requested))
        .collect::<Vec<_>>();
    // The admitted run stops at its next heartbeat; the queued one ends at once.
    assert_eq!(
        canceled,
        [
            ("run-1", RunState::Dispatching, true),
            ("run-2", RunState::Canceled, true)
        ]
    );
    assert_eq!(
        store
            .runs()
            .retry_run(OWNER_ID, revision.workflow().repository_id(), "run-2", 13)
            .await
            .unwrap_err()
            .message,
        NATIVE_RUNS_UNAVAILABLE
    );
}

#[tokio::test]
async fn removal_turns_waiting_checks_into_configuration_errors() {
    let store = postgres_store();
    store
        .native_runs()
        .add_account("owner", None, 1)
        .await
        .unwrap();
    let revision = workflow("owner/repo");
    for (request_id, head_oid) in [("waiting", "a"), ("settled", "b")] {
        let head_oid = head_oid.repeat(40);
        store
            .requests()
            .start_request(StartRequestInput {
                id: request_id.into(),
                repo_id: "owner/repo".into(),
                name: request_id.into(),
                author_user_id: "user_public".into(),
                title: None,
                author_role: RequestActorRole::Public,
                audience: RequestAudience::Public,
                base_main_oid: head_oid.clone(),
                event_id: format!("{request_id}-event"),
                now_unix: 2,
            })
            .await
            .unwrap();
        let evaluation = if request_id == "waiting" {
            RequestCheckEvaluation::awaiting_approval(
                request_id,
                &head_oid,
                vec![RequestCheck::Native(NativeRequestCheck::for_revision(
                    &revision,
                ))],
                3,
            )
        } else {
            RequestCheckEvaluation::no_checks(request_id, &head_oid, 3)
        }
        .unwrap();
        store
            .requests()
            .record_request_checks(RecordRequestChecksCommand {
                evaluation,
                revisions: vec![revision.clone()],
                runs: Vec::new(),
            })
            .await
            .unwrap();
    }

    let removal = store
        .native_runs()
        .remove_account("owner", 10)
        .await
        .unwrap();
    assert_eq!(removal.withdrawn_evaluations.len(), 1);
    assert_eq!(
        removal
            .repositories
            .iter()
            .map(|repository| repository.repository_id())
            .collect::<Vec<_>>(),
        ["owner/repo"]
    );
    let waiting = store
        .requests()
        .request_check_evaluation("waiting", &"a".repeat(40))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        waiting.state,
        RequestCheckEvaluationState::ConfigurationError
    );
    assert_eq!(waiting.message.as_deref(), Some(NATIVE_RUNS_UNAVAILABLE));
    let settled = store
        .requests()
        .request_check_evaluation("settled", &"b".repeat(40))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(settled.state, RequestCheckEvaluationState::NoChecks);

    // A head evaluated before the removal cannot record a wait after it.
    let refused = store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::awaiting_approval(
                "settled",
                "c".repeat(40),
                vec![RequestCheck::Native(NativeRequestCheck::for_revision(
                    &revision,
                ))],
                11,
            )
            .unwrap(),
            revisions: vec![revision],
            runs: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(refused.message, NATIVE_RUNS_UNAVAILABLE);
}
