use super::{
    AuthorHistory, AuthorOutcome, AuthorRating, AuthorRecord, AuthorRecordTotals,
    AuthorRequestFacts, AuthoredRequest, author_record_subject, author_request_outcomes,
};
use crate::{
    repository::access::{RepositoryAccess, RepositoryActor},
    requests::{
        RequestActorRole, RequestState,
        fixtures::{access, actor_views, agent},
    },
    views::ViewId,
};

const OPEN: AuthoredRequest<'static> = AuthoredRequest {
    author_user_id: Some("author"),
    author_role: RequestActorRole::Public,
    state: RequestState::Open,
};

fn owner() -> RepositoryAccess {
    access(RepositoryActor::Owner, ViewId::private(), true)
}

fn record(request: AuthoredRequest<'_>, totals: AuthorRecordTotals) -> Option<AuthorRecord> {
    author_record_subject(request, &owner(), actor_views())
        .map(|subject| subject.record(totals).unwrap())
}

fn history(request: AuthoredRequest<'_>, totals: AuthorRecordTotals) -> AuthorHistory {
    match record(request, totals) {
        Some(AuthorRecord::History(history)) => history,
        other => panic!("expected a history, got {other:?}"),
    }
}

#[test]
fn only_full_view_maintainers_see_a_record_and_only_for_contributors_with_an_account() {
    for author_role in [RequestActorRole::Member, RequestActorRole::Owner] {
        let request = AuthoredRequest {
            author_role,
            ..OPEN
        };
        assert!(author_record_subject(request, &owner(), actor_views()).is_none());
    }
    let deleted_author = AuthoredRequest {
        author_user_id: None,
        ..OPEN
    };
    assert!(author_record_subject(deleted_author, &owner(), actor_views()).is_none());
    for viewer in [
        RepositoryAccess::public(),
        access(RepositoryActor::Member, agent(), true),
    ] {
        assert!(author_record_subject(OPEN, &viewer, actor_views()).is_none());
    }
    let member = access(RepositoryActor::Member, ViewId::private(), true);
    assert!(author_record_subject(OPEN, &member, actor_views()).is_some());
}

#[test]
fn a_request_with_nothing_else_submitted_is_the_first() {
    let own_open = AuthorRecordTotals {
        open: 1,
        ..AuthorRecordTotals::default()
    };
    assert_eq!(record(OPEN, own_open), Some(AuthorRecord::FirstRequest));
    let draft = AuthoredRequest {
        state: RequestState::Draft,
        ..OPEN
    };
    assert_eq!(
        record(draft, AuthorRecordTotals::default()),
        Some(AuthorRecord::FirstRequest)
    );
}

#[test]
fn an_open_request_is_left_out_of_its_own_open_count() {
    let totals = AuthorRecordTotals {
        open: 3,
        ..AuthorRecordTotals::default()
    };
    assert_eq!(history(OPEN, totals).other_open, 2);
    let draft = AuthoredRequest {
        state: RequestState::Draft,
        ..OPEN
    };
    assert_eq!(history(draft, totals).other_open, 3);
}

#[test]
fn a_rating_needs_possible_totals() {
    let rated = |rating_score_sum, rating_count| {
        author_record_subject(OPEN, &owner(), actor_views())
            .unwrap()
            .record(AuthorRecordTotals {
                open: 1,
                rating_score_sum,
                rating_count,
                ..AuthorRecordTotals::default()
            })
    };
    assert_eq!(
        rated(9, 2).unwrap(),
        AuthorRecord::History(AuthorHistory {
            merged: 0,
            closed_by_maintainer: 0,
            withdrawn: 0,
            other_open: 0,
            rating: Some(AuthorRating {
                score_sum: 9,
                count: 2,
            }),
        })
    );
    assert!(rated(0, 1).is_err());
    assert!(rated(6, 1).is_err());
    assert!(rated(1, 0).is_err());
}

#[test]
fn a_counted_request_has_one_outcome_and_a_missing_closer_is_a_maintainer_closure() {
    let open = AuthorRequestFacts {
        submitted: true,
        authored_as_contributor: true,
        merged: false,
        closed: false,
        closed_by_author: false,
    };
    let closed = AuthorRequestFacts {
        closed: true,
        ..open
    };
    for (facts, outcome) in [
        (open, Some(AuthorOutcome::Open)),
        (
            AuthorRequestFacts {
                merged: true,
                ..open
            },
            Some(AuthorOutcome::Merged),
        ),
        (closed, Some(AuthorOutcome::ClosedByMaintainer)),
        (
            AuthorRequestFacts {
                closed_by_author: true,
                ..closed
            },
            Some(AuthorOutcome::Withdrawn),
        ),
        (
            AuthorRequestFacts {
                submitted: false,
                ..open
            },
            None,
        ),
        (
            AuthorRequestFacts {
                authored_as_contributor: false,
                merged: true,
                ..open
            },
            None,
        ),
    ] {
        assert_eq!(
            author_request_outcomes(&facts).collect::<Vec<_>>(),
            Vec::from_iter(outcome),
            "{facts:?}"
        );
    }
}
