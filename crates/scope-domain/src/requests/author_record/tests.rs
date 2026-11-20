use super::{
    AuthorHistory, AuthorRating, AuthorRecord, AuthorRecordTotals, AuthoredRequest,
    author_record_subject,
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
