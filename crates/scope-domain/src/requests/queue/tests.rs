use super::*;
use crate::error::DomainErrorKind;
use crate::requests::RequestActorRole;
use crate::views::ViewId;

#[test]
fn groups_split_active_rows_by_who_acts_next() {
    use RequestAttentionReason as Reason;
    use RequestQueueGroup::{NeedsYou, Waiting};
    for (reason, expected) in [
        (Reason::Invited, NeedsYou),
        (Reason::Claimed, NeedsYou),
        (Reason::NewActivity, NeedsYou),
        (Reason::Restored, NeedsYou),
        (Reason::SnoozeExpired, NeedsYou),
        (Reason::Authored, Waiting),
        (Reason::Waiting, Waiting),
        (Reason::Open, Waiting),
        (Reason::ClaimedElsewhere, Waiting),
        (Reason::Unclaimed, Waiting),
        (Reason::Snoozed, Waiting),
        (Reason::Settled, Waiting),
        (Reason::Closed, Waiting),
        (Reason::Merged, Waiting),
    ] {
        assert_eq!(
            request_queue_group(RequestQueueSection::Active, reason, false),
            expected,
            "{reason:?}"
        );
    }
    assert_eq!(
        request_queue_group(RequestQueueSection::Active, Reason::Authored, true),
        NeedsYou
    );
    for (section, expected) in [
        (RequestQueueSection::Unclaimed, RequestQueueGroup::Unclaimed),
        (RequestQueueSection::SetAside, RequestQueueGroup::SetAside),
        (RequestQueueSection::Done, RequestQueueGroup::Done),
    ] {
        assert_eq!(
            request_queue_group(section, Reason::Settled, true),
            expected
        );
    }
}

#[test]
fn personal_set_aside_precedes_retained_claim_and_other_activity_reactivates() {
    let request = open_request();
    let waiting = attention(&request, RequestAttentionState::Waiting);
    let claim = RequestClaim {
        request_id: request.id.clone(),
        claimer_user_id: "maintainer".into(),
        claimed_at_unix: 1,
        updated_at_unix: 1,
    };
    let classified = classify_request_queue_item(RequestQueueFacts {
        request_state: request.state(),
        request_activity_version: request.activity_version,
        request_author_user_id: request.author_user_id.as_deref(),
        viewer_user_id: Some("maintainer"),
        viewer_is_maintainer: true,
        viewer_is_invitee: false,
        attention: Some(&waiting),
        claim: Some(&claim),
        now_unix: 10,
    });
    assert_eq!(classified.section, RequestQueueSection::SetAside);

    assert!(reactivate_request_attention(&waiting, "maintainer", 4, 11).is_none());
    let active = reactivate_request_attention(&waiting, "author", 4, 11).unwrap();
    assert_eq!(active.state, RequestAttentionState::Active);
    assert_eq!(active.reason, RequestAttentionReason::NewActivity);
}

#[test]
fn snooze_expiry_is_active_without_rewriting_the_checkpoint() {
    let request = open_request();
    let mut snoozed = attention(&request, RequestAttentionState::Snoozed);
    snoozed.reason = RequestAttentionReason::Snoozed;
    snoozed.snoozed_until_unix = Some(10);
    let classified = classify_request_queue_item(RequestQueueFacts {
        request_state: request.state(),
        request_activity_version: request.activity_version,
        request_author_user_id: request.author_user_id.as_deref(),
        viewer_user_id: Some("maintainer"),
        viewer_is_maintainer: true,
        viewer_is_invitee: false,
        attention: Some(&snoozed),
        claim: None,
        now_unix: 10,
    });
    assert_eq!(classified.section, RequestQueueSection::Active);
    assert_eq!(classified.reason, RequestAttentionReason::SnoozeExpired);
    assert_eq!(
        reactivate_request_attention(&snoozed, "author", 4, 9)
            .unwrap()
            .reason,
        RequestAttentionReason::NewActivity
    );
}

#[test]
fn another_maintainers_claim_is_set_aside_and_cannot_be_reclaimed() {
    let request = open_request();
    let claim = RequestClaim {
        request_id: request.id.clone(),
        claimer_user_id: "other-maintainer".into(),
        claimed_at_unix: 1,
        updated_at_unix: 1,
    };
    let classified = classify_request_queue_item(RequestQueueFacts {
        request_state: request.state(),
        request_activity_version: request.activity_version,
        request_author_user_id: request.author_user_id.as_deref(),
        viewer_user_id: Some("maintainer"),
        viewer_is_maintainer: true,
        viewer_is_invitee: false,
        attention: None,
        claim: Some(&claim),
        now_unix: 10,
    });
    assert_eq!(classified.section, RequestQueueSection::SetAside);
    assert_eq!(classified.reason, RequestAttentionReason::ClaimedElsewhere);
    assert!(!classified.can_claim);
    assert!(!classified.can_set_aside);

    let error = apply_request_attention_action(ApplyRequestAttentionInput {
        request: &request,
        actor_user_id: "maintainer",
        actor_is_maintainer: true,
        expected_activity_version: request.activity_version,
        existing_attention: None,
        existing_claim: Some(&claim),
        action: RequestAttentionAction::Claim,
        now_unix: 10,
    })
    .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);
}

#[test]
fn release_requires_the_viewers_own_claim_and_returns_the_request_to_unclaimed() {
    let request = open_request();
    let claim = RequestClaim {
        request_id: request.id.clone(),
        claimer_user_id: "maintainer".into(),
        claimed_at_unix: 1,
        updated_at_unix: 1,
    };
    let classified = classify_request_queue_item(RequestQueueFacts {
        request_state: request.state(),
        request_activity_version: request.activity_version,
        request_author_user_id: request.author_user_id.as_deref(),
        viewer_user_id: Some("maintainer"),
        viewer_is_maintainer: true,
        viewer_is_invitee: false,
        attention: None,
        claim: Some(&claim),
        now_unix: 10,
    });
    assert_eq!(classified.reason, RequestAttentionReason::Claimed);
    assert!(classified.can_release);

    let released = apply_request_attention_action(ApplyRequestAttentionInput {
        request: &request,
        actor_user_id: "maintainer",
        actor_is_maintainer: true,
        expected_activity_version: request.activity_version,
        existing_attention: None,
        existing_claim: Some(&claim),
        action: RequestAttentionAction::Release,
        now_unix: 10,
    })
    .unwrap();
    assert_eq!(
        released,
        RequestAttentionMutation {
            attention: None,
            claim: None
        }
    );

    for existing_claim in [None, Some(&claim)] {
        let error = apply_request_attention_action(ApplyRequestAttentionInput {
            request: &request,
            actor_user_id: "other-maintainer",
            actor_is_maintainer: true,
            expected_activity_version: request.activity_version,
            existing_attention: None,
            existing_claim,
            action: RequestAttentionAction::Release,
            now_unix: 10,
        })
        .unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::Conflict);
    }
}

#[test]
fn restore_requires_a_current_set_aside_state() {
    let request = open_request();
    let active = attention(&request, RequestAttentionState::Active);
    let error = apply_request_attention_action(ApplyRequestAttentionInput {
        request: &request,
        actor_user_id: "maintainer",
        actor_is_maintainer: true,
        expected_activity_version: request.activity_version,
        existing_attention: Some(&active),
        existing_claim: None,
        action: RequestAttentionAction::Restore,
        now_unix: 10,
    })
    .unwrap_err();
    assert_eq!(error.kind, DomainErrorKind::Conflict);
}

#[test]
fn every_attention_write_advances_the_revision() {
    let request = open_request();
    let settle = |existing: Option<&RequestAttention>| {
        apply_request_attention_action(ApplyRequestAttentionInput {
            request: &request,
            actor_user_id: "maintainer",
            actor_is_maintainer: true,
            expected_activity_version: request.activity_version,
            existing_attention: existing,
            existing_claim: None,
            action: RequestAttentionAction::Settle,
            now_unix: 10,
        })
        .unwrap()
        .attention
        .unwrap()
    };
    let first = settle(None);
    assert_eq!(first.revision, 1);
    let reactivated =
        reactivate_request_attention(&first, "someone-else", request.activity_version + 1, 11)
            .unwrap();
    assert_eq!(reactivated.revision, 2);
    assert_eq!(settle(Some(&reactivated)).revision, 3);
}

#[test]
fn reply_wait_capability_requires_a_maintainer_and_an_open_request() {
    use crate::repository::access::{RepositoryAccess, RepositoryActor};
    use crate::requests::{RequestViewer, request_policy};
    for state in [
        RequestState::Draft,
        RequestState::Open,
        RequestState::Closed,
        RequestState::Merged,
    ] {
        let mut request = open_request();
        request.submitted_at_unix = (state != RequestState::Draft).then_some(1);
        request.closed_at_unix = (state == RequestState::Closed).then_some(2);
        request.merged_at_unix = (state == RequestState::Merged).then_some(2);
        for actor in [
            RepositoryActor::Owner,
            RepositoryActor::Member,
            RepositoryActor::Public,
        ] {
            let mut access = RepositoryAccess::public();
            access.actor = actor;
            let permissions = request_policy(
                &request,
                RequestViewer::new(access, Some("author"), false),
                &crate::views::Views::builtin(),
            )
            .permissions;
            assert!(permissions.can_reply_to_discussion);
            assert_eq!(
                permissions.can_wait_after_reply,
                actor != RepositoryActor::Public && state == RequestState::Open
            );
        }
    }
}

fn attention(request: &Request, state: RequestAttentionState) -> RequestAttention {
    RequestAttention {
        request_id: request.id.clone(),
        user_id: "maintainer".into(),
        state,
        reason: RequestAttentionReason::Waiting,
        through_activity_version: request.activity_version,
        snoozed_until_unix: None,
        updated_at_unix: 1,
        revision: 1,
    }
}

fn open_request() -> Request {
    Request {
        id: "request".into(),
        repo_id: "repo".into(),
        name: "request".into(),
        author_user_id: Some("author".into()),
        author_role: RequestActorRole::Public,
        view: ViewId::public(),
        base_main_oid: "0".repeat(40),
        head_oid: "1".repeat(40),
        git_snapshot: None,
        title: "Request".into(),
        description_markdown: String::new(),
        activity_version: 3,
        submitted_at_unix: Some(1),
        closed_at_unix: None,
        closed_by_user_id: None,
        merged_at_unix: None,
        merged_by_user_id: None,
        merged_head_oid: None,
        merged_main_oid: None,
        created_at_unix: 1,
        updated_at_unix: 1,
    }
}
