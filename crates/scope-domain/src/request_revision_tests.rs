use super::requests::{
    fixtures::{source_blob, start_input},
    *,
};
use crate::views::ViewId;

fn uploaded_request() -> Request {
    let started = start_request(
        StartRequestFacts::default(),
        StartRequestInput {
            id: "request_change".to_string(),
            name: "change".to_string(),
            title: Some("Change".to_string()),
            author_view: ViewId::private(),
            view: ViewId::private(),
            ..start_input(RequestActorRole::Owner)
        },
        &crate::views::Views::builtin(),
    )
    .unwrap();
    record_working_request_upload(
        started.request,
        RecordWorkingRequestUploadInput {
            request_id: "request_change".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: "head-1".to_string(),
            git_snapshot: source_blob("head-1"),
            now_unix: 11,
        },
    )
    .unwrap()
    .request
}

fn revise(git_facts: RequestRevisionGitFacts) -> RequestRevisionMutation {
    record_request_revision(
        uploaded_request(),
        false,
        RecordRequestRevisionInput {
            request_id: "request_change".to_string(),
            actor_user_id: "author".to_string(),
            actor_can_edit: true,
            expected_old_head_oid: Some("head-1".to_string()),
            new_head_oid: "head-2".to_string(),
            git_snapshot: source_blob("head-2"),
            git_facts,
            event_id: "event_revision".to_string(),
            body: None,
            now_unix: 12,
        },
    )
    .unwrap()
}

fn extending_facts(contained_main_oid: Option<&str>, descends: bool) -> RequestRevisionGitFacts {
    RequestRevisionGitFacts {
        contains_old_head: true,
        contained_main_oid: contained_main_oid.map(str::to_string),
        contained_main_descends_from_base: descends,
    }
}

#[test]
fn revision_records_snapshot_without_manufacturing_a_discussion() {
    let mutation = revise(extending_facts(None, false));

    assert_eq!(mutation.orphan_objects, vec![source_blob("head-1")]);
    assert_eq!(mutation.revision.old_head_oid, "head-1");
    assert_eq!(mutation.revision.new_head_oid, "head-2");
    assert_eq!(mutation.revision.id, mutation.event.id);
    assert!(!mutation.revision.rewrote_history);
    assert_eq!(mutation.revision.commits_after_oid(), "head-1");
}

#[test]
fn revision_moves_the_base_to_newer_main_the_head_contains() {
    let mutation = revise(extending_facts(Some("main-2"), true));

    assert_eq!(mutation.request.base_main_oid, "main-2");
    assert_eq!(mutation.revision.base_main_oid, "main-2");
}

#[test]
fn revision_keeps_the_base_when_contained_main_does_not_descend_from_it() {
    for facts in [
        extending_facts(None, false),
        extending_facts(Some("unrelated-main"), false),
    ] {
        let mutation = revise(facts);

        assert_eq!(mutation.request.base_main_oid, "base");
        assert_eq!(mutation.revision.base_main_oid, "base");
    }
}

#[test]
fn rewritten_revision_lists_its_commits_after_the_new_base() {
    let mutation = revise(RequestRevisionGitFacts {
        contains_old_head: false,
        contained_main_oid: Some("main-2".to_string()),
        contained_main_descends_from_base: true,
    });

    assert!(mutation.revision.rewrote_history);
    assert_eq!(mutation.revision.old_head_oid, "head-1");
    assert_eq!(mutation.revision.commits_after_oid(), "main-2");
}

#[test]
fn review_revision_is_newest_unless_an_existing_revision_is_pinned() {
    let revisions = vec![revision("revision-2", 2), revision("revision-1", 1)];

    assert_eq!(
        select_request_review_revision(&revisions, None)
            .unwrap()
            .unwrap()
            .id,
        "revision-2"
    );
    assert_eq!(
        select_request_review_revision(&revisions, Some("revision-1"))
            .unwrap()
            .unwrap()
            .id,
        "revision-1"
    );
    assert!(select_request_review_revision(&revisions, Some("missing")).is_err());
}

fn revision(id: &str, position: u64) -> RequestRevision {
    RequestRevision {
        id: id.to_string(),
        request_id: "request".to_string(),
        position,
        actor_user_id: Some("author".to_string()),
        old_head_oid: "old".to_string(),
        new_head_oid: "new".to_string(),
        base_main_oid: "base".to_string(),
        rewrote_history: false,
        git_snapshot: source_blob(id),
        created_at_unix: position,
    }
}
