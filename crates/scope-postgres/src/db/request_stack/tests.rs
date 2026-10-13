use super::*;
use crate::db::{
    CloseRequestCommand, MetadataStore, RequestRevisionContextPublication, SubmitRequestCommand,
};
use scope_domain::{
    requests::{
        RecordRequestRevisionInput, RequestActorRole, RequestAutoMergeStopReason,
        RequestRevisionGitFacts, RequestStackStatus, StartRequestInput,
    },
    views::ViewId,
};
use sea_orm::TransactionTrait;

async fn load(store: &MetadataStore, id: &str) -> Request {
    store.requests().request_by_id(id).await.unwrap().unwrap()
}

fn input(request: &Request, head: &str, event: &str, now: u64) -> RecordRequestRevisionInput {
    let encoded = if head.len() == 40 && head.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        head.to_string()
    } else {
        use sha2::{Digest, Sha256};
        Sha256::digest(head.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()[..40]
            .to_string()
    };
    let head = encoded.as_str();
    RecordRequestRevisionInput {
        new_base: None,
        new_base_stack_depth: None,
        request_id: request.id.clone(),
        actor_user_id: "user_owner".into(),
        actor_can_edit: true,
        expected_old_head_oid: Some(request.head_oid.clone()),
        new_head_oid: head.into(),
        git_snapshot: scope_domain::content::SourceBlob {
            content_ref: scope_domain::content_ref::ContentRef::git_bundle_sha256(format!(
                "sha256-{head}"
            )),
            sha256: format!("sha256-{head}"),
            git_oid: head.into(),
            git_file_mode: scope_domain::content::DEFAULT_GIT_FILE_MODE.into(),
            size_bytes: 1,
        },
        git_facts: RequestRevisionGitFacts {
            contains_old_head: true,
            contained_main_oid: None,
            contained_main_descends_from_base: false,
        },
        event_id: event.into(),
        body: None,
        now_unix: now,
    }
}

async fn parent_facts(
    store: &MetadataStore,
    request: &Request,
) -> Option<RequestBasePublicationFacts> {
    let RequestBase::Request { request_id, .. } = &request.base else {
        return None;
    };
    let parent = load(store, request_id).await;
    let revision = store
        .requests()
        .latest_request_revision(request_id)
        .await
        .unwrap()
        .unwrap();
    Some(RequestBasePublicationFacts::Request(
        RequestParentPublicationFacts {
            request_id: parent.id,
            revision_id: revision.id,
            head_oid: parent.head_oid,
            base_version: parent.base_version,
            parent_contains_pin: true,
            child_contains_pin: true,
        },
    ))
}

async fn start(store: &MetadataStore, id: &str, base: RequestBase, now: u64) -> Request {
    store
        .requests()
        .start_request(StartRequestInput {
            id: id.into(),
            repo_id: "owner/repo".into(),
            name: id.into(),
            author_user_id: "user_owner".into(),
            title: None,
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base,
            main_ancestor_oid: "base".into(),
            event_id: format!("start_{id}"),
            now_unix: now,
        })
        .await
        .unwrap();
    let request = load(store, id).await;
    let mutation = store
        .requests()
        .record_request_revision_with_stack(
            input(
                &request,
                &format!("head_{id}"),
                &format!("revision_{id}"),
                now + 1,
            ),
            request.base_version,
            parent_facts(store, &request).await,
            Vec::new(),
        )
        .await
        .unwrap();
    store
        .requests()
        .submit_request(SubmitRequestCommand {
            request_id: id.into(),
            actor_user_id: "user_owner".into(),
            event_id: format!("submit_{id}"),
            now_unix: now + 2,
        })
        .await
        .unwrap();
    load(store, &mutation.request.id).await
}

async fn pin(store: &MetadataStore, parent: &Request) -> RequestBase {
    RequestBase::Request {
        request_id: parent.id.clone(),
        revision_id: store
            .requests()
            .latest_request_revision(&parent.id)
            .await
            .unwrap()
            .unwrap()
            .id,
        head_oid: parent.head_oid.clone(),
    }
}

fn fact(parent_head_oid: &str, child: &Request, contains: bool) -> RequestStackChildFacts {
    RequestStackChildFacts {
        parent_head_oid: parent_head_oid.to_string(),
        request_id: child.id.clone(),
        head_oid: child.head_oid.clone(),
        base_version: child.base_version,
        parent_contains_pin: contains,
        contained_main_oid: None,
        contained_main_descends_from_ancestor: false,
    }
}

#[tokio::test]
async fn parent_publication_requires_complete_child_evidence_and_preserves_extended_revision() {
    let store = super::super::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let grandchild = start(&store, "grandchild", pin(&store, &child).await, 9).await;
    let revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let error = store
        .requests()
        .record_request_revision_with_stack(
            input(&parent, "parent_extended", "extend", 10),
            parent.base_version,
            None,
            Vec::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(load(&store, "parent").await.head_oid, parent.head_oid);
    let publication = input(&parent, "parent_extended", "extend", 10);
    let child_facts = vec![fact(&publication.new_head_oid, &child, true)];
    store
        .requests()
        .record_request_revision_with_stack(publication, parent.base_version, None, child_facts)
        .await
        .unwrap();
    let extended = load(&store, "child").await;
    assert_eq!(extended.stack_status, RequestStackStatus::ParentExtended);
    assert_eq!(extended.base, child.base);
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        revision
    );
    let mut stale_repin = input(&child, &child.head_oid, "stale_repin", 12);
    stale_repin.new_base = Some(child.base.clone());
    let stale_error = store
        .requests()
        .record_request_revision_with_stack(
            stale_repin,
            child.base_version,
            parent_facts(&store, &child).await,
            vec![fact(&child.head_oid, &grandchild, true)],
        )
        .await
        .unwrap_err();
    assert_eq!(stale_error.kind, crate::error::PostgresErrorKind::Conflict);
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        revision
    );
    let current_parent = load(&store, "parent").await;
    let publication = input(&current_parent, "rewritten_parent", "rewrite", 13);
    let child_facts = vec![fact(&publication.new_head_oid, &child, false)];
    store
        .requests()
        .record_request_revision_with_stack(
            publication,
            current_parent.base_version,
            None,
            child_facts,
        )
        .await
        .unwrap();
    assert_eq!(
        load(&store, &child.id).await.stack_status,
        RequestStackStatus::ParentRewritten
    );
    assert_eq!(
        load(&store, &grandchild.id).await.stack_status,
        RequestStackStatus::AncestorBlocked
    );
    let blocked_base = pin(&store, &child).await;
    let blocked = store
        .requests()
        .start_request(StartRequestInput {
            id: "blocked_child".into(),
            repo_id: "owner/repo".into(),
            name: "blocked-child".into(),
            author_user_id: "user_owner".into(),
            title: None,
            author_role: RequestActorRole::Owner,
            author_view: ViewId::private(),
            view: ViewId::private(),
            base: blocked_base,
            main_ancestor_oid: "base".into(),
            event_id: "blocked_started".into(),
            now_unix: 14,
        })
        .await
        .unwrap();
    assert_eq!(
        blocked.request.stack_status,
        RequestStackStatus::AncestorBlocked
    );
    store
        .requests()
        .close_request(CloseRequestCommand {
            request_id: parent.id,
            actor_user_id: "user_owner".into(),
            event_id: "close".into(),
            now_unix: 15,
        })
        .await
        .unwrap();
    let closed = load(&store, "child").await;
    assert_eq!(closed.stack_status, RequestStackStatus::ParentClosed);
    assert_eq!(closed.base, child.base);
}

#[tokio::test]
async fn committed_context_receipt_survives_parent_close_and_rejects_changed_payload() {
    let store = super::super::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let publication = input(&parent, "parent_extended", "extend", 10);
    let child_facts = vec![fact(&publication.new_head_oid, &child, true)];
    let parent = store
        .requests()
        .record_request_revision_with_stack(publication, parent.base_version, None, child_facts)
        .await
        .unwrap()
        .request;
    let revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let context = RequestRevisionContextPublication {
        operation_key: "repin".into(),
        expected_revision_id: revision.id,
        expected_head_oid: child.head_oid.clone(),
        expected_base_version: child.base_version,
        base: pin(&store, &parent).await,
        expected_parent_base_version: Some(parent.base_version),
    };
    let mut publication = input(&child, &child.head_oid, "repin_event", 11);
    publication.new_base = Some(context.base.clone());
    let accepted = store
        .requests()
        .record_request_revision_context(
            publication.clone(),
            context.clone(),
            parent_facts(&store, &child).await,
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(accepted.revision.new_head_oid, revision.new_head_oid);
    assert_ne!(accepted.revision.id, context.expected_revision_id);
    store
        .requests()
        .close_request(CloseRequestCommand {
            request_id: parent.id,
            actor_user_id: "user_owner".into(),
            event_id: "close".into(),
            now_unix: 12,
        })
        .await
        .unwrap();
    let receipt = store
        .requests()
        .request_revision_context_receipt(&child.id, "user_owner", &context)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(receipt.revision, accepted.revision);
    assert_eq!(
        receipt.request.stack_status,
        RequestStackStatus::ParentClosed
    );
    let mut changed = context;
    changed.expected_parent_base_version =
        changed.expected_parent_base_version.map(|value| value + 1);
    let error = store
        .requests()
        .request_revision_context_receipt(&child.id, "user_owner", &changed)
        .await
        .unwrap_err();
    assert_eq!(error.kind, crate::error::PostgresErrorKind::Conflict);
}

#[tokio::test]
async fn containing_merge_retargets_atomically_without_replacing_child_revision() {
    let store = super::super::requests::tests::postgres_store();
    let parent = start(
        &store,
        "parent",
        RequestBase::Main {
            review_base_oid: "base".into(),
        },
        2,
    )
    .await;
    let child = start(&store, "child", pin(&store, &parent).await, 6).await;
    let grandchild = start(&store, "grandchild", pin(&store, &child).await, 10).await;
    let revision = store
        .requests()
        .latest_request_revision(&child.id)
        .await
        .unwrap()
        .unwrap();
    let context =
        scope_domain::requests::RequestCheckContext::for_revision(&child, &revision).unwrap();
    let evaluation =
        scope_domain::requests::RequestCheckEvaluation::no_checks(&context, 13).unwrap();
    store
        .requests()
        .record_request_checks(crate::db::RecordRequestChecksCommand {
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
            evaluation: evaluation.clone(),
            revisions: Vec::new(),
            runs: Vec::new(),
            push_to_github: false,
        })
        .await
        .unwrap();
    let merged = scope_domain::requests::merge_request(
        &parent,
        scope_domain::requests::MergeRequestInput {
            request_id: parent.id.clone(),
            actor_user_id: "user_owner".into(),
            actor_is_maintainer: true,
            landing: scope_domain::requests::MergeRequestLanding::NativeMerge,
            merged_head_oid: parent.head_oid.clone(),
            merged_main_oid: "landed_main".into(),
            merged_event_id: "merge".into(),
            now_unix: 14,
        },
    )
    .unwrap();
    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
        .await
        .unwrap();
    lock_graph(&tx, &parent, None).await.unwrap();
    super::super::request_lifecycle_effects::persist_lifecycle_mutation(
        &tx,
        &merged.request,
        &merged.events,
    )
    .await
    .unwrap();
    let mut stale = fact(&parent.head_oid, &child, true);
    stale.base_version += 1;
    assert!(
        retarget_children(&tx, &merged.request, &[stale], 14)
            .await
            .is_err()
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        load(&store, "parent").await.state(),
        scope_domain::requests::RequestState::Open
    );
    let tx = store.db.begin().await.unwrap();
    acquire_aggregate_lock(&tx, "repository", &parent.repo_id)
        .await
        .unwrap();
    lock_graph(&tx, &parent, None).await.unwrap();
    super::super::request_lifecycle_effects::persist_lifecycle_mutation(
        &tx,
        &merged.request,
        &merged.events,
    )
    .await
    .unwrap();
    retarget_children(
        &tx,
        &merged.request,
        &[fact(&parent.head_oid, &child, true)],
        14,
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let retargeted = load(&store, "child").await;
    assert_eq!(
        retargeted.base,
        RequestBase::Main {
            review_base_oid: parent.head_oid
        }
    );
    assert_eq!(retargeted.base_version, child.base_version + 1);
    assert_eq!(retargeted.head_oid, child.head_oid);
    assert_eq!(retargeted.stack_depth, 1);
    let retained = store
        .requests()
        .request_check_evaluation(&child.id, &child.head_oid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retained.id, evaluation.id);
    assert_eq!(retained.revision_id, evaluation.revision_id);
    assert_eq!(retained.tested_oid, evaluation.tested_oid);
    assert_eq!(retained.base_version, retargeted.base_version);
    assert!(retained.matches_request(&retargeted));
    assert_eq!(load(&store, &grandchild.id).await.stack_depth, 2);
    assert_eq!(
        store
            .requests()
            .latest_request_revision(&child.id)
            .await
            .unwrap()
            .unwrap(),
        revision
    );
}

mod github_pushes;
mod integration_conflict;

mod auto_merge;
mod carried;
mod reparent;
