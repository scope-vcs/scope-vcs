use super::*;

pub(super) async fn request_check_context(store: &crate::db::MetadataStore) -> RequestCheckContext {
    let requests = store.requests();
    let request = requests.request_for_tests("req_1").await.unwrap().unwrap();
    let revision = requests
        .request_revision_window("req_1", None, 1)
        .await
        .unwrap()
        .revisions
        .into_iter()
        .next()
        .unwrap();
    RequestCheckContext::for_revision(&request, &revision).unwrap()
}

pub(super) async fn open_request_with_revision(store: &crate::db::MetadataStore) -> String {
    start_public_request(store).await;
    store
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            event_id: "submitted".into(),
            now_unix: 4,
        })
        .await
        .unwrap();
    store
        .requests()
        .record_request_revision(RecordRequestRevisionInput {
            new_base: None,
            new_base_stack_depth: None,
            request_id: "req_1".into(),
            actor_user_id: "user_public".into(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head".into()),
            new_head_oid: "a".repeat(40),
            git_snapshot: SourceBlob {
                content_ref: ContentRef::git_bundle_sha256("sha256-auto-merge"),
                sha256: "sha256-auto-merge".into(),
                git_oid: "a".repeat(40),
                git_file_mode: DEFAULT_GIT_FILE_MODE.into(),
                size_bytes: 1,
            },
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: Some("f".repeat(40)),
                contained_main_descends_from_base: true,
            },
            event_id: "revision_current".into(),
            body: None,
            now_unix: 5,
        })
        .await
        .unwrap()
        .revision
        .id
}

pub(super) fn authorize(
    intent_id: &str,
    event_id: &str,
    revision_id: &str,
    now_unix: u64,
) -> AuthorizeRequestAutoMergeCommand {
    AuthorizeRequestAutoMergeCommand {
        request_id: "req_1".into(),
        actor_user_id: "user_owner".into(),
        expected_revision_id: revision_id.into(),
        expected_head_oid: "a".repeat(40),
        intent_id: intent_id.into(),
        event_id: event_id.into(),
        now_unix,
    }
}

pub(super) async fn record_terminal_check_evidence(
    store: &crate::db::MetadataStore,
    succeeded: bool,
) {
    store
        .native_runs()
        .add_account("owner", None, 1)
        .await
        .unwrap();
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/check.yml",
        br#"
name: Check
on:
  manual: true
container:
  image: alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 5m
jobs:
  check:
    steps:
      - name: Check
        run: echo ok
"#,
    )
    .unwrap()
    .into_revision("owner/repo")
    .unwrap();
    let run = Run::new(
        "run-check-evidence",
        "run-check-evidence",
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some("user_owner".into()),
        RunSource::ephemeral_git_bundle(source_blob(&"a".repeat(40), &"b".repeat(64), 1)).unwrap(),
        5,
    )
    .unwrap();
    store
        .runs()
        .enqueue_run(run, revision.clone())
        .await
        .unwrap();
    store
        .native_runs()
        .add_account("owner", None, 5)
        .await
        .unwrap();
    let token = "c".repeat(64);
    let claim = store
        .runs()
        .dispatch_job(
            "run-check-evidence",
            "check",
            "attempt-evidence",
            &token,
            "runtime",
            5,
            20,
        )
        .await
        .unwrap();
    store
        .runs()
        .start_attempt_step(&claim.attempt.id, &token, 0, 5)
        .await
        .unwrap();
    store
        .runs()
        .complete_attempt_step(
            &claim.attempt.id,
            &token,
            0,
            if succeeded {
                scope_domain::runs::step::StepConclusion::Succeeded
            } else {
                scope_domain::runs::step::StepConclusion::Failed { exit_code: 1 }
            },
            false,
            6,
        )
        .await
        .unwrap();
    if succeeded {
        store
            .runs()
            .complete_attempt(
                &claim.attempt.id,
                &token,
                scope_domain::runs::step::AttemptConclusion::Succeeded,
                false,
                6,
            )
            .await
            .unwrap();
    }
    store
        .runs()
        .complete_cloud_task_absence(&claim.attempt.id, 6)
        .await
        .unwrap();

    let mut check = NativeRequestCheck::for_revision(&revision);
    check.run_id = Some("run-check-evidence".into());
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
            evaluation: RequestCheckEvaluation::started(
                &request_check_context(store).await,
                vec![RequestCheck::Native(check)],
                6,
            )
            .unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
}

pub(super) async fn wait_for_blocked_transaction_count(
    store: &crate::db::MetadataStore,
    blocker_pid: i32,
    expected: i64,
) {
    tokio::time::timeout(std::time::Duration::from_secs(60), async {
        loop {
            let count = store
                .db
                .query_one_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    "WITH RECURSIVE blocked(pid) AS ( \
                         SELECT pid FROM pg_stat_activity \
                          WHERE $1 = ANY(pg_blocking_pids(pid)) \
                         UNION \
                         SELECT activity.pid FROM pg_stat_activity activity \
                           JOIN blocked ON blocked.pid = ANY(pg_blocking_pids(activity.pid)) \
                     ) SELECT count(*) AS count FROM blocked",
                    [blocker_pid.into()],
                ))
                .await
                .unwrap()
                .unwrap()
                .try_get::<i64>("", "count")
                .unwrap();
            if count >= expected {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("expected transactions blocked by the request guard");
}
