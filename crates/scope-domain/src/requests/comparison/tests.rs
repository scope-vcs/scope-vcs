use super::*;
use crate::requests::{RequestBase, fixtures::source_blob};

fn revision() -> RequestRevision {
    RequestRevision {
        id: "revision".into(),
        request_id: "request".into(),
        position: 2,
        actor_user_id: Some("author".into()),
        old_head_oid: "previous-head".into(),
        new_head_oid: "head".into(),
        main_ancestor_oid: "main-ancestor".into(),
        base: RequestBase::Main {
            review_base_oid: "saved-review-base".into(),
        },
        rewrote_history: false,
        git_snapshot: source_blob("head"),
        created_at_unix: 1,
    }
}

#[test]
fn request_review_and_push_history_have_distinct_boundaries() {
    let mut revision = revision();
    let request = RequestComparison::request(&revision, ViewId::private());
    let push = RequestComparison::push(&revision, ViewId::private());
    assert_eq!(
        request.membership().excluded_ancestor_oids()[0],
        "saved-review-base"
    );
    assert_eq!(
        push.membership().excluded_ancestor_oids()[0],
        "previous-head"
    );
    assert_eq!(request.membership().head_oid(), "head");
    assert_eq!(push.membership().head_oid(), "head");
    revision.rewrote_history = true;
    let rewrite = RequestComparison::push(&revision, ViewId::private());
    assert_eq!(
        rewrite.membership().excluded_ancestor_oids()[0],
        "saved-review-base"
    );
    revision.main_ancestor_oid = "new-main".into();
    assert_eq!(
        request,
        RequestComparison::request(&revision, ViewId::private())
    );
}

#[test]
fn membership_requires_an_existing_head_ancestor_outside_the_boundary() {
    let range = RequestCommitRange::push(&revision());
    for (exists, ancestor_of_head, ancestor_of_excluded_boundary, expected) in [
        (true, true, false, true),
        (false, true, false, false),
        (true, false, false, false),
        (true, true, true, false),
    ] {
        let facts = RequestCommitMembershipFacts {
            exists,
            ancestor_of_head,
            ancestor_of_excluded_boundary,
        };
        assert_eq!(range.contains_commit(facts), expected);
    }
}

#[test]
fn commit_comparison_uses_its_first_parent_and_keeps_revision_and_view_identity() {
    let revision = revision();
    let comparison = RequestComparison::push(&revision, ViewId::public());
    let membership = RequestCommitMembershipFacts {
        exists: true,
        ancestor_of_head: true,
        ancestor_of_excluded_boundary: false,
    };
    for first_parent in [None, Some("first-parent".to_string())] {
        let commit = comparison
            .commit("member-commit".into(), membership, first_parent.clone())
            .unwrap();
        assert_eq!(
            commit.identity(),
            &RequestComparisonIdentity {
                request_id: "request".into(),
                revision_id: "revision".into(),
                mode: RequestComparisonMode::Commit,
                view: ViewId::public(),
                base_oid: first_parent,
                head_oid: "member-commit".into(),
            }
        );
    }
    let outside_range = RequestCommitMembershipFacts {
        ancestor_of_excluded_boundary: true,
        ..membership
    };
    assert!(
        comparison
            .commit("old-commit".into(), outside_range, Some("parent".into()))
            .is_err()
    );
    assert_ne!(
        comparison.identity(),
        RequestComparison::push(&revision, ViewId::private()).identity()
    );
}

#[test]
fn extending_push_excludes_new_parent_history_as_well_as_previous_child_head() {
    let mut revision = revision();
    revision.base = RequestBase::Request {
        request_id: "parent".into(),
        revision_id: "parent-revision".into(),
        head_oid: "new-parent-head".into(),
    };
    let comparison = RequestComparison::push(&revision, ViewId::private());
    assert_eq!(
        comparison.membership().excluded_ancestor_oids(),
        &["previous-head", "new-parent-head"]
    );
    let parent_commit = RequestCommitMembershipFacts {
        exists: true,
        ancestor_of_head: true,
        ancestor_of_excluded_boundary: true,
    };
    assert!(!comparison.membership().contains_commit(parent_commit));
}
