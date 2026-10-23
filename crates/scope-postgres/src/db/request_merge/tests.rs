mod file_rules;
mod landed;
mod push_paths;

use crate::db::{
    AuthorizeRequestAutoMergeCommand, CancelRequestAutoMergeCommand,
    ClaimDueRequestAutoMergesCommand, ExpectedRequestAutoMerge, MergeRequestContentCommand,
    RecordRequestChecksCommand,
    requests::tests::{
        postgres_store, start_public_request,
        terminal_effects::{assert_terminal_effects, seed_terminal_effects},
    },
};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    landing_file::RepositoryLandingFileMutation,
    policy::ScopePath,
    repository::{
        git::{GitHead, GitPoolPack, GitPoolPackOwner, GitPoolPackStorage},
        updates::RequestMergeOrigin,
    },
    requests::{
        GITHUB_CHECK_MERGE_FRESHNESS_SECS, RecordRequestRevisionInput, RequestCheckEvaluation,
        RequestRevisionGitFacts, RequestState,
    },
    reviewed_updates::content::{
        ReviewedContentChange, ReviewedUpdateInput, apply_reviewed_update_to_repo,
    },
    runs::catalog::RepositoryWorkflowCatalog,
};

const BASE_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REQUEST_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const CHECK_HEAD: &str = "dddddddddddddddddddddddddddddddddddddddd";
const MERGED_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";
const GITHUB_READ_AT: u64 = 5;

#[tokio::test]
async fn locked_merge_derives_maintainer_authorization_before_content_persistence() {
    let store = merge_store().await;
    let prepared = merge_preparation(&store).await;

    let error = store
        .requests()
        .merge_request_content(merge_command(&store, "user_public", prepared).await)
        .await
        .unwrap_err();

    assert_eq!(
        error.kind,
        crate::error::PostgresErrorKind::PermissionDenied
    );
    assert_eq!(error.message, "repo maintainer required");
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repo.git_head.unwrap().head_oid, BASE_HEAD);
    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state(),
        RequestState::Open
    );
}

#[tokio::test]
async fn locked_merge_allows_owner_and_persists_content_and_request_once() {
    let store = merge_store().await;
    let prepared = merge_preparation(&store).await;
    seed_terminal_effects(&store).await;

    let mutation = store
        .requests()
        .merge_request_content(merge_command(&store, "user_owner", prepared).await)
        .await
        .unwrap();

    assert_eq!(mutation.request.request.state(), RequestState::Merged);
    assert_eq!(mutation.git_head.head_oid, MERGED_HEAD);
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repo.git_head.unwrap().head_oid, MERGED_HEAD);
    assert_eq!(repo.graph.commits.len(), 2);
    assert_terminal_effects(&store, "event_merged_user_owner").await;
}

#[tokio::test]
async fn cancellation_after_preparation_fences_the_final_content_commit() {
    let store = merge_store().await;
    store
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: "event_merge_revision".into(),
            expected_head_oid: REQUEST_HEAD.into(),
            intent_id: "intent_cancel_race".into(),
            event_id: "event_auto_merge_enabled".into(),
            now_unix: 7,
        })
        .await
        .unwrap();
    let claim = store
        .requests()
        .claim_due_request_auto_merges(ClaimDueRequestAutoMergesCommand {
            now_unix: 7,
            lease_expires_at_unix: 100,
            limit: 1,
        })
        .await
        .unwrap()
        .pop()
        .unwrap();
    let prepared = merge_preparation(&store).await;
    store
        .requests()
        .cancel_request_auto_merge(CancelRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_intent_id: claim.intent.id.clone(),
            event_id: "event_auto_merge_cancelled".into(),
            now_unix: 8,
        })
        .await
        .unwrap();

    let mut command = merge_command(&store, "user_owner", prepared).await;
    command.now_unix = 9;
    command.expected_auto_merge = Some(ExpectedRequestAutoMerge {
        intent_id: claim.intent.id,
        revision_id: claim.intent.revision_id,
        head_oid: claim.intent.head_oid,
        claim_token: claim.claim_token,
        fulfilled_event_id: "event_auto_merge_fulfilled".into(),
    });
    let error = store
        .requests()
        .merge_request_content(command)
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(error.message, "auto-merge claim is no longer current");
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repo.git_head.unwrap().head_oid, BASE_HEAD);
    assert_eq!(repo.graph.commits.len(), 1);
    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state(),
        RequestState::Open
    );
}

#[tokio::test]
async fn stale_manual_preparation_does_not_stop_a_current_auto_merge_intent() {
    let store = merge_store().await;
    store
        .requests()
        .authorize_request_auto_merge(AuthorizeRequestAutoMergeCommand {
            request_id: "req_1".into(),
            actor_user_id: "user_owner".into(),
            expected_revision_id: "event_merge_revision".into(),
            expected_head_oid: REQUEST_HEAD.into(),
            intent_id: "intent_after_manual_prepare".into(),
            event_id: "event_auto_merge_enabled_after_manual_prepare".into(),
            now_unix: 7,
        })
        .await
        .unwrap();
    let prepared = merge_preparation(&store).await;
    let mut stale_manual = merge_command(&store, "user_owner", prepared).await;
    stale_manual.expected_request_head_oid = BASE_HEAD.into();
    stale_manual.now_unix = 8;

    let error = store
        .requests()
        .merge_request_content(stale_manual)
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(
        store
            .requests()
            .request_auto_merge_intent("req_1")
            .await
            .unwrap()
            .unwrap()
            .status,
        scope_domain::requests::RequestAutoMergeIntentStatus::Active
    );
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(repo.git_head.unwrap().head_oid, BASE_HEAD);
    assert_eq!(repo.graph.commits.len(), 1);
}

#[derive(Clone)]
struct MergePreparation {
    expected_git_frontier: scope_domain::repository::git::GitFrontier,
    expected_repo_change_version: u64,
    update: ReviewedUpdateInput,
    workflow_catalog: RepositoryWorkflowCatalog,
}

async fn merge_store() -> super::super::MetadataStore {
    merge_store_with_github(false).await
}

async fn merge_store_with_github(github: bool) -> super::super::MetadataStore {
    let store = postgres_store();
    let mut repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    let initial_update = reviewed_update(
        &repo,
        BASE_HEAD,
        1,
        None,
        "/.scope/RULES.md",
        source_blob("rules-content"),
    );
    apply_reviewed_update_to_repo(&mut repo, initial_update).unwrap();
    store
        .repositories()
        .replace_repository_for_tests(repo)
        .await
        .unwrap();
    start_public_request(&store).await;
    store
        .requests()
        .mutate_request_for_tests("req_1", |request| {
            request.base = scope_domain::requests::RequestBase::Main {
                review_base_oid: BASE_HEAD.into(),
            };
            request.main_ancestor_oid = BASE_HEAD.into();
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
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
            new_head_oid: REQUEST_HEAD.into(),
            git_snapshot: SourceBlob {
                git_oid: REQUEST_HEAD.into(),
                ..source_blob(REQUEST_HEAD)
            },
            git_facts: RequestRevisionGitFacts {
                contains_old_head: true,
                contained_main_oid: None,
                contained_main_descends_from_base: false,
            },
            event_id: "event_merge_revision".into(),
            body: None,
            now_unix: 5,
        })
        .await
        .unwrap();
    let request = store
        .requests()
        .request_for_tests("req_1")
        .await
        .unwrap()
        .unwrap();
    let revision = super::super::request_revision_rows::latest_revision_for_request(
        store.db.as_ref(),
        "req_1",
    )
    .await
    .unwrap()
    .unwrap();
    let context =
        scope_domain::requests::RequestCheckContext::for_revision(&request, &revision).unwrap();
    let evaluation = if github {
        use sea_orm::ConnectionTrait;
        store.db.execute_unprepared("INSERT INTO scope_github_connections (repo_id, installation_id, github_repository_id, github_full_name, connected_by_user_id, connected_at_unix, status) VALUES ('owner/repo', 7, 42, 'octo/repo', 'user_owner', 1, 'Connected')").await.unwrap();
        let names = vec!["test".to_string()];
        let mut evaluation = scope_domain::requests::RequestCheckPlan::evaluate_github(
            &request,
            &context,
            &scope_domain::views::Views::builtin(),
            scope_domain::requests::GitHubRequestChecks {
                github_repository_id: 42,
                tested: scope_domain::requests::GitHubTestedCommit::CheckCommit {
                    oid: CHECK_HEAD.into(),
                    base: scope_domain::requests::CheckCommitBase::new(BASE_HEAD, BASE_HEAD)
                        .unwrap(),
                },
                required_check_names: &names,
            },
            None,
            5,
        )
        .unwrap()
        .evaluation;
        evaluation.approve(Vec::new(), 5).unwrap();
        evaluation
    } else {
        RequestCheckEvaluation::no_checks(&context, 5).unwrap()
    };
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            repository_incarnation: store
                .repositories()
                .repository_record("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .incarnation(),
            expected_canonical_main_oid: store
                .requests()
                .request_check_base("owner/repo")
                .await
                .unwrap(),
            evaluation,
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    store
}

async fn merge_preparation(store: &super::super::MetadataStore) -> MergePreparation {
    let repo = store
        .repositories()
        .repository_for_tests("owner/repo")
        .await
        .unwrap()
        .unwrap();
    let update = reviewed_update(
        &repo,
        MERGED_HEAD,
        2,
        Some(BASE_HEAD),
        "/README.md",
        source_blob("merged-content"),
    );
    let workflow_catalog = RepositoryWorkflowCatalog::captured(
        "owner/repo",
        MERGED_HEAD,
        repo.record.change_version + 1,
        Vec::new(),
    )
    .unwrap();
    MergePreparation {
        expected_git_frontier: repo.git_head.as_ref().unwrap().frontier(),
        expected_repo_change_version: repo.record.change_version,
        update,
        workflow_catalog,
    }
}

fn reviewed_update(
    repo: &scope_domain::repository::Repository,
    head_oid: &str,
    sequence: u64,
    base_oid: Option<&str>,
    path: &str,
    content: SourceBlob,
) -> ReviewedUpdateInput {
    ReviewedUpdateInput {
        occurred_at_unix: None,
        branch: "refs/heads/main".to_string(),
        author_id: "user_owner".to_string(),
        message: format!("update {head_oid}"),
        git_head: GitHead::new(
            head_oid.to_string(),
            sequence,
            repo.record.change_version + 1,
        ),
        main_pack: GitPoolPack {
            id: format!("main-pack-{head_oid}"),
            owner: GitPoolPackOwner::Main {
                first_sequence: sequence,
                last_sequence: sequence,
            },
            head_oid: head_oid.to_string(),
            prerequisites: base_oid.map(str::to_string).into_iter().collect(),
            storage: GitPoolPackStorage::Inline {
                sealed: head_oid.as_bytes().to_vec(),
            },
        },
        changes: vec![ReviewedContentChange {
            path: ScopePath::parse(path).unwrap(),
            content: Some(content),
        }],
        previous_config: Some(repo.repo_config.clone()),
        config: repo.repo_config.clone(),
        open_requests_by_view: Default::default(),
    }
}

fn source_blob(label: &str) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(format!("sha256-{label}")),
        sha256: format!("sha256-{label}"),
        git_oid: scope_domain::content_ref::git_blob_oid(label.as_bytes()),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

async fn merge_command(
    store: &super::super::MetadataStore,
    actor_user_id: &str,
    prepared: MergePreparation,
) -> MergeRequestContentCommand {
    MergeRequestContentCommand {
        stack_children: Vec::new(),
        owner: "owner".to_string(),
        name: "repo".to_string(),
        request_id: "req_1".to_string(),
        actor_user_id: actor_user_id.to_string(),
        merged_event_id: format!("event_merged_{actor_user_id}"),
        expected_git_frontier: prepared.expected_git_frontier,
        expected_repo_change_version: prepared.expected_repo_change_version,
        expected_request_head_oid: REQUEST_HEAD.to_string(),
        expected_auto_merge: None,
        expected_merge_evidence: {
            let request = store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap();
            store
                .requests()
                .request_merge_evidence(
                    &request.id,
                    &request.head_oid,
                    request.base_version,
                    request.base.review_base_oid(),
                )
                .await
                .unwrap()
        },
        update: prepared.update,
        landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
        workflow_catalog: prepared.workflow_catalog,
        origin: RequestMergeOrigin::Canonical {
            request_id: "req_1".to_string(),
            request_head_oid: REQUEST_HEAD.to_string(),
        },
        now_unix: 5,
    }
}

fn github_commit() -> crate::db::GitHubCheckCommit {
    crate::db::GitHubCheckCommit {
        repo_id: "owner/repo".into(),
        github_repository_id: 42,
        commit_oid: CHECK_HEAD.into(),
    }
}

async fn apply_github_result(store: &super::super::MetadataStore, passed: bool) {
    use scope_domain::requests::{GitHubCheckConclusion, GitHubCheckRun, GitHubCheckStatus};
    let commit = github_commit();
    let read = store
        .requests()
        .start_github_check_read(&commit)
        .await
        .unwrap();
    store
        .requests()
        .apply_github_check_read(
            &commit,
            read,
            GITHUB_READ_AT,
            &[GitHubCheckRun {
                commit_oid: CHECK_HEAD.into(),
                name: "test".into(),
                github_check_run_id: read,
                status: GitHubCheckStatus::Completed,
                conclusion: Some(if passed {
                    GitHubCheckConclusion::Success
                } else {
                    GitHubCheckConclusion::Failure
                }),
                details_url: None,
                check_suite_id: None,
            }],
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn manual_merge_rejects_check_reads_that_changed_after_preparation() {
    for change in ["in-flight", "failed", "new-passing-read", "expired"] {
        let store = merge_store_with_github(true).await;
        apply_github_result(&store, true).await;
        let prepared = merge_preparation(&store).await;
        let mut command = merge_command(&store, "user_owner", prepared).await;
        match change {
            "in-flight" => {
                store
                    .requests()
                    .start_github_check_read(&github_commit())
                    .await
                    .unwrap();
            }
            "failed" => apply_github_result(&store, false).await,
            "new-passing-read" => apply_github_result(&store, true).await,
            "expired" => command.now_unix = 65,
            _ => unreachable!(),
        }
        let error = store
            .requests()
            .merge_request_content(command)
            .await
            .unwrap_err();
        assert_eq!(
            error.kind,
            crate::error::PostgresErrorKind::Conflict,
            "{change}"
        );
        let repo = store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(repo.git_head.unwrap().head_oid, BASE_HEAD, "{change}");
        assert_eq!(
            store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap()
                .state(),
            RequestState::Open,
            "{change}"
        );
    }
}

#[tokio::test]
async fn final_merge_fences_check_reads_and_rejects_expiry_while_publication_waits() {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
    for expires in [false, true] {
        let store = merge_store_with_github(true).await;
        apply_github_result(&store, true).await;
        let prepared = merge_preparation(&store).await;
        let mut command = merge_command(&store, "user_owner", prepared).await;
        if expires {
            command.now_unix = GITHUB_READ_AT + GITHUB_CHECK_MERGE_FRESHNESS_SECS - 3;
        }
        let held = store.db.begin().await.unwrap();
        let holder_pid: i32 = held
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT pg_backend_pid() AS pid",
            ))
            .await
            .unwrap()
            .unwrap()
            .try_get("", "pid")
            .unwrap();
        held.execute_unprepared(
            "SELECT id FROM scope_repositories WHERE id = 'owner/repo' FOR UPDATE",
        )
        .await
        .unwrap();
        let merge_store = store.clone();
        let merge =
            tokio::spawn(
                async move { merge_store.requests().merge_request_content(command).await },
            );
        let merge_pid = super::super::locks::wait_for_transaction_waiter(&store, holder_pid).await;
        let read_store = store.clone();
        let read = tokio::spawn(async move {
            read_store
                .requests()
                .start_github_check_read(&github_commit())
                .await
        });
        super::super::locks::wait_for_transaction_waiter(&store, merge_pid).await;
        assert!(!read.is_finished());
        if expires {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
        held.commit().await.unwrap();
        let result = merge.await.unwrap();
        if expires {
            assert_eq!(
                result.unwrap_err().kind,
                crate::error::PostgresErrorKind::Conflict
            );
        } else {
            assert_eq!(
                result.unwrap().request.request.state(),
                RequestState::Merged
            );
        }
        assert_eq!(read.await.unwrap().unwrap(), 2);
        assert_eq!(
            store
                .requests()
                .request_for_tests("req_1")
                .await
                .unwrap()
                .unwrap()
                .state(),
            if expires {
                RequestState::Open
            } else {
                RequestState::Merged
            }
        );
        assert_eq!(
            store
                .repositories()
                .repository_for_tests("owner/repo")
                .await
                .unwrap()
                .unwrap()
                .git_head
                .unwrap()
                .head_oid,
            if expires { BASE_HEAD } else { MERGED_HEAD }
        );
    }
}

#[tokio::test]
async fn a_batched_merge_whose_checks_expire_before_the_batch_commits_does_not_land() {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement, TransactionTrait};
    let store = merge_store_with_github(true).await;
    apply_github_result(&store, true).await;
    let prepared = merge_preparation(&store).await;
    let mut fresh = merge_command(&store, "user_owner", prepared).await;
    fresh.now_unix = GITHUB_READ_AT + GITHUB_CHECK_MERGE_FRESHNESS_SECS - 3;
    store
        .requests()
        .start_request(scope_domain::requests::StartRequestInput {
            id: "req_2".to_string(),
            name: "slow-neighbor".to_string(),
            event_id: "event_started_req_2".to_string(),
            ..crate::db::requests::tests::public_start_input()
        })
        .await
        .unwrap();
    let mut neighbor = fresh.clone();
    neighbor.request_id = "req_2".to_string();
    neighbor.merged_event_id = "event_merged_req_2".to_string();

    let held = store.db.begin().await.unwrap();
    let holder_pid: i32 = held
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT pg_backend_pid() AS pid",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "pid")
        .unwrap();
    super::super::acquire_aggregate_lock(&held, "request", "req_2")
        .await
        .unwrap();
    let merge_store = store.clone();
    let batch = tokio::spawn(async move {
        merge_store
            .requests()
            .merge_request_contents(vec![fresh, neighbor])
            .await
    });
    super::super::locks::wait_for_transaction_waiter(&store, holder_pid).await;
    tokio::time::sleep(std::time::Duration::from_secs(4)).await;
    held.commit().await.unwrap();

    let outcome = batch.await.unwrap().unwrap();
    assert!(outcome.landed.is_empty());
    assert_eq!(
        outcome.rejected.unwrap().kind,
        crate::error::PostgresErrorKind::Conflict
    );
    assert_eq!(
        store
            .requests()
            .request_for_tests("req_1")
            .await
            .unwrap()
            .unwrap()
            .state(),
        RequestState::Open
    );
    assert_eq!(
        store
            .repositories()
            .repository_for_tests("owner/repo")
            .await
            .unwrap()
            .unwrap()
            .git_head
            .unwrap()
            .head_oid,
        BASE_HEAD
    );
}
