use super::RequestRevision;
use crate::requests::{Request, fixtures};
use crate::views::Views;

fn revision() -> RequestRevision {
    RequestRevision {
        base_version: 1,
        id: "revision_1".into(),
        request_id: "request_1".into(),
        position: 2,
        actor_user_id: Some("author".into()),
        old_head_oid: "base".into(),
        new_head_oid: "head".into(),
        base: crate::requests::RequestBase::Main {
            review_base_oid: "base".into(),
        },
        main_ancestor_oid: "base".into(),
        rewrote_history: false,
        created_at_unix: 2,
    }
}

fn restorable(request: &Request) -> bool {
    revision()
        .restoration_pack_owner(request, &Views::builtin())
        .is_ok()
}

#[test]
fn only_requests_that_can_no_longer_change_accept_restored_packs() {
    let open = fixtures::open_request();
    let closed = Request {
        closed_at_unix: Some(30),
        ..open.clone()
    };
    let merged = Request {
        merged_at_unix: Some(30),
        ..open.clone()
    };
    assert!(!restorable(&fixtures::pushed_draft(
        crate::requests::RequestActorRole::Public
    )));
    assert!(!restorable(&open));
    assert!(restorable(&closed));
    assert!(restorable(&merged));
}
