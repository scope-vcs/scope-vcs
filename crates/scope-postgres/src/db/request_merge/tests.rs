use crate::db::{
    AuthorizeRequestAutoMergeCommand, CancelRequestAutoMergeCommand,
    ClaimDueRequestAutoMergesCommand, ExpectedRequestAutoMerge, MergeRequestContentCommand,
    RecordRequestChecksCommand,
    requests::tests::{postgres_store, start_public_request},
};
use scope_domain::{
    content::{DEFAULT_GIT_FILE_MODE, SourceBlob},
    content_ref::ContentRef,
    landing_file::RepositoryLandingFileMutation,
    policy::ScopePath,
    repository::{
        git::{GitHead, GitPackSpan},
        updates::RequestMergeOrigin,
    },
    requests::{RecordRequestRevisionInput, RequestCheckEvaluation, RequestState},
    reviewed_updates::content::{
        ReviewedContentChange, ReviewedUpdateInput, apply_reviewed_update_to_repo,
    },
    runs::catalog::RepositoryWorkflowCatalog,
};

const BASE_HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const REQUEST_HEAD: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const MERGED_HEAD: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[tokio::test]
async fn locked_merge_derives_maintainer_authorization_before_content_persistence() {
    let store = merge_store().await;
    let prepared = merge_preparation(&store).await;

    let error = store
        .requests()
        .merge_request_content(
            merge_command("user_public", prepared),
            &super::super::generated_ids::test_generated_id,
        )
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

    let mutation = store
        .requests()
        .merge_request_content(
            merge_command("user_owner", prepared),
            &super::super::generated_ids::test_generated_id,
        )
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
}

#[tokio::test]
async fn cancellation_after_preparation_fences_the_final_content_commit() {
    let store = merge_store().await;
    store
        .requests()
        .record_request_checks(RecordRequestChecksCommand {
            evaluation: RequestCheckEvaluation::no_checks("req_1", REQUEST_HEAD, 6).unwrap(),
            revisions: Vec::new(),
            runs: Vec::new(),
        })
        .await
        .unwrap();
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
        .claim_due_request_auto_merges(
            ClaimDueRequestAutoMergesCommand {
                now_unix: 7,
                lease_expires_at_unix: 100,
                limit: 1,
            },
            &super::super::generated_ids::test_generated_id,
        )
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

    let mut command = merge_command("user_owner", prepared);
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
        .merge_request_content(command, &super::super::generated_ids::test_generated_id)
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
    let mut stale_manual = merge_command("user_owner", prepared);
    stale_manual.expected_request_head_oid = BASE_HEAD.into();
    stale_manual.now_unix = 8;

    let error = store
        .requests()
        .merge_request_content(
            stale_manual,
            &super::super::generated_ids::test_generated_id,
        )
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

struct MergePreparation {
    expected_git_frontier: scope_domain::repository::git::GitFrontier,
    expected_repo_change_version: u64,
    update: ReviewedUpdateInput,
    workflow_catalog: RepositoryWorkflowCatalog,
}

async fn merge_store() -> super::super::MetadataStore {
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
    stage_segment(&store, &initial_update.git_pack_span.segment).await;
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
            request.submitted_at_unix = Some(4);
            request.updated_at_unix = 4;
        })
        .await
        .unwrap();
    store
        .requests()
        .record_request_revision(
            RecordRequestRevisionInput {
                request_id: "req_1".into(),
                actor_user_id: "user_public".into(),
                actor_can_edit: true,
                expected_old_head_oid: Some("head".into()),
                new_head_oid: REQUEST_HEAD.into(),
                git_snapshot: source_blob(REQUEST_HEAD),
                event_id: "event_merge_revision".into(),
                body: None,
                now_unix: 5,
            },
            &super::super::generated_ids::test_generated_id,
        )
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
    stage_segment(store, &update.git_pack_span.segment).await;
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

async fn stage_segment(
    store: &super::super::MetadataStore,
    segment: &scope_domain::repository::git::GitSegmentRef,
) {
    let repositories = store.repositories();
    repositories
        .begin_git_segment_upload(
            "owner/repo",
            &segment.segment_id,
            &format!("git/segments/v2/owner/repo/{}", segment.segment_id),
            segment.encoding_version,
            1,
        )
        .await
        .unwrap();
    repositories
        .mark_git_segment_upload_ready(segment, 2, 2)
        .await
        .unwrap();
}

fn reviewed_update(
    repo: &scope_domain::repository::Repository,
    head_oid: &str,
    sequence: u64,
    base_oid: Option<&str>,
    path: &str,
    content: SourceBlob,
) -> ReviewedUpdateInput {
    let segment = scope_domain::repository::git::GitSegmentRef {
        segment_id: format!("segment-{head_oid}"),
        sha256: "c".repeat(64),
        plaintext_bytes: 1,
        encoding_version: 2,
    };
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
        git_pack_span: GitPackSpan {
            first_sequence: sequence,
            last_sequence: sequence,
            geometric_tier: 0,
            base_oid: base_oid.map(str::to_string),
            head_oid: head_oid.to_string(),
            segment,
        },
        changes: vec![ReviewedContentChange {
            path: ScopePath::parse(path).unwrap(),
            content: Some(content),
        }],
        previous_config: Some(repo.repo_config.clone()),
        config: repo.repo_config.clone(),
    }
}

fn source_blob(label: &str) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::git_bundle_sha256(format!("sha256-{label}")),
        sha256: format!("sha256-{label}"),
        git_oid: label.to_string(),
        git_file_mode: DEFAULT_GIT_FILE_MODE.to_string(),
        size_bytes: 1,
    }
}

fn merge_command(actor_user_id: &str, prepared: MergePreparation) -> MergeRequestContentCommand {
    MergeRequestContentCommand {
        owner: "owner".to_string(),
        name: "repo".to_string(),
        request_id: "req_1".to_string(),
        actor_user_id: actor_user_id.to_string(),
        merged_event_id: format!("event_merged_{actor_user_id}"),
        expected_git_frontier: prepared.expected_git_frontier,
        expected_repo_change_version: prepared.expected_repo_change_version,
        expected_request_head_oid: REQUEST_HEAD.to_string(),
        expected_auto_merge: None,
        update: prepared.update,
        landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
        workflow_catalog: prepared.workflow_catalog,
        origin: RequestMergeOrigin::Private {
            request_id: "req_1".to_string(),
            request_head_oid: REQUEST_HEAD.to_string(),
        },
        now_unix: 5,
    }
}
