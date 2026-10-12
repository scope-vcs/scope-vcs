use crate::{
    db::{
        DispatchAdmission, MetadataStore, RecordRequestChecksCommand,
        requests::tests::postgres_store,
        test_support::fixtures::{repository, source_blob, store_with_repositories, user},
    },
    error::PostgresErrorKind,
};
use scope_domain::{
    requests::{
        NativeRequestCheck, RecordRequestRevisionInput, RequestActorRole, RequestCheck,
        RequestCheckContext, RequestCheckEvaluation, RequestCheckEvaluationState,
        RequestRevisionGitFacts, StartRequestInput,
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
    views::ViewId,
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
        ViewId::private(),
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
                author_view: ViewId::public(),
                view: ViewId::public(),
                main_ancestor_oid: head_oid.clone(),
                event_id: format!("{request_id}-event"),
                now_unix: 2,
            })
            .await
            .unwrap();
        record_check_revision(&store, request_id, &head_oid, 3).await;
        let context = saved_check_context(&store, request_id, &head_oid).await;
        let evaluation = if request_id == "waiting" {
            RequestCheckEvaluation::awaiting_approval(
                &context,
                vec![RequestCheck::Native(NativeRequestCheck::for_revision(
                    &revision,
                ))],
                3,
            )
        } else {
            RequestCheckEvaluation::no_checks(&context, 3)
        }
        .unwrap();
        store
            .requests()
            .record_request_checks(RecordRequestChecksCommand {
                expected_canonical_main_oid: store
                    .requests()
                    .request_check_base("owner/repo")
                    .await
                    .unwrap(),
                repository_incarnation: store
                    .repositories()
                    .repository_record("owner/repo")
                    .await
                    .unwrap()
                    .unwrap()
                    .incarnation(),
                evaluation,
                revisions: vec![revision.clone()],
                runs: Vec::new(),
                push_to_github: false,
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

    record_check_revision(&store, "settled", &"c".repeat(40), 11).await;

    let refused = store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            evaluation: RequestCheckEvaluation::awaiting_approval(
                &saved_check_context(&store, "settled", &"c".repeat(40)).await,
                vec![RequestCheck::Native(NativeRequestCheck::for_revision(
                    &revision,
                ))],
                11,
            )
            .unwrap(),
            revisions: vec![revision],
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap_err();
    assert_eq!(refused.message, NATIVE_RUNS_UNAVAILABLE);
}

async fn saved_check_context(
    store: &MetadataStore,
    request_id: &str,
    head_oid: &str,
) -> RequestCheckContext {
    let request = store
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    let revision = store
        .requests()
        .latest_request_revision(request_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.head_oid, head_oid);
    RequestCheckContext::for_revision(&request, &revision).unwrap()
}

async fn record_check_revision(
    store: &MetadataStore,
    request_id: &str,
    head_oid: &str,
    now_unix: u64,
) {
    let request = store
        .requests()
        .request_by_id(request_id)
        .await
        .unwrap()
        .unwrap();
    store
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: request_id.into(),
                actor_user_id: "user_public".into(),
                actor_can_edit: true,
                expected_old_head_oid: Some(request.head_oid),
                new_head_oid: head_oid.into(),
                git_snapshot: source_blob(head_oid, &"b".repeat(64), 1),
                git_facts: RequestRevisionGitFacts {
                    contains_old_head: true,
                    contained_main_oid: None,
                    contained_main_descends_from_base: false,
                },
                event_id: format!("{request_id}-revision-{now_unix}"),
                body: None,
                now_unix,
            },
            &crate::db::generated_ids::test_generated_id,
        )
        .await
        .unwrap();
}
