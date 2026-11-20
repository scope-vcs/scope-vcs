use super::{
    AuthorHistory, AuthorOutcome, AuthorRating, AuthorRecord, AuthorRecordTotals,
    AuthorRequestFacts, AuthoredRequest, author_record_subject, author_request_outcomes,
};
use crate::{
    repository::access::{RepositoryAccess, RepositoryActor},
    requests::{
        RequestState,
        fixtures::{access, actor_views, agent},
    },
    views::ViewId,
};

const OPEN: AuthoredRequest<'static> = AuthoredRequest {
    author_user_id: Some("author"),
    state: RequestState::Open,
};

fn owner() -> RepositoryAccess {
    access(RepositoryActor::Owner, ViewId::private(), true)
}

fn record(request: AuthoredRequest<'_>, totals: AuthorRecordTotals) -> Option<AuthorRecord> {
    author_record_subject(request, &owner(), actor_views())?
        .record(&RepositoryAccess::public(), totals)
        .unwrap()
}

fn history(request: AuthoredRequest<'_>, totals: AuthorRecordTotals) -> AuthorHistory {
    match record(request, totals) {
        Some(AuthorRecord::History(history)) => history,
        other => panic!("expected a history, got {other:?}"),
    }
}

#[test]
fn readers_of_the_full_view_see_the_record_of_authors_who_do_not_read_it() {
    let full_view_member = access(RepositoryActor::Member, ViewId::private(), true);
    let off_the_full_view = [
        RepositoryAccess::public(),
        access(RepositoryActor::Member, agent(), true),
        access(RepositoryActor::Member, ViewId::public(), false),
    ];
    for viewer in [owner(), full_view_member.clone()] {
        for author in &off_the_full_view {
            let subject = author_record_subject(OPEN, &viewer, actor_views()).unwrap();
            assert_eq!(
                subject.record(author, AuthorRecordTotals::default()),
                Ok(Some(AuthorRecord::FirstRequest)),
                "{author:?}"
            );
        }
        for author in [owner(), full_view_member.clone()] {
            let subject = author_record_subject(OPEN, &viewer, actor_views()).unwrap();
            assert_eq!(
                subject.record(&author, AuthorRecordTotals::default()),
                Ok(None),
                "{author:?}"
            );
        }
    }
    for viewer in &off_the_full_view {
        assert!(author_record_subject(OPEN, viewer, actor_views()).is_none());
    }
    let deleted_author = AuthoredRequest {
        author_user_id: None,
        ..OPEN
    };
    assert!(author_record_subject(deleted_author, &owner(), actor_views()).is_none());
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
    let merged_their_own = AuthorRecordTotals {
        merged_by_author: 1,
        ..own_open
    };
    assert_eq!(
        record(OPEN, merged_their_own),
        Some(AuthorRecord::History(AuthorHistory {
            merged: 0,
            closed: 0,
            withdrawn: 0,
            other_open: 0,
            rating: None,
        }))
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
            .record(
                &RepositoryAccess::public(),
                AuthorRecordTotals {
                    open: 1,
                    rating_score_sum,
                    rating_count,
                    ..AuthorRecordTotals::default()
                },
            )
    };
    assert_eq!(
        rated(9, 2).unwrap(),
        Some(AuthorRecord::History(AuthorHistory {
            merged: 0,
            closed: 0,
            withdrawn: 0,
            other_open: 0,
            rating: Some(AuthorRating {
                score_sum: 9,
                count: 2,
            }),
        }))
    );
    assert!(rated(0, 1).is_err());
    assert!(rated(6, 1).is_err());
    assert!(rated(1, 0).is_err());
}

#[test]
fn a_submitted_request_has_one_outcome_and_a_missing_actor_is_someone_else() {
    let open = AuthorRequestFacts {
        submitted: true,
        merged: false,
        merged_by_author: false,
        closed: false,
        closed_by_author: false,
    };
    let merged = AuthorRequestFacts {
        merged: true,
        ..open
    };
    let closed = AuthorRequestFacts {
        closed: true,
        ..open
    };
    for (facts, outcome) in [
        (open, Some(AuthorOutcome::Open)),
        (merged, Some(AuthorOutcome::Merged)),
        (
            AuthorRequestFacts {
                merged_by_author: true,
                ..merged
            },
            Some(AuthorOutcome::MergedByAuthor),
        ),
        (closed, Some(AuthorOutcome::Closed)),
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
    ] {
        assert_eq!(
            author_request_outcomes(&facts).collect::<Vec<_>>(),
            Vec::from_iter(outcome),
            "{facts:?}"
        );
    }
}
