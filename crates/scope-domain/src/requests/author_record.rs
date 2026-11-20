use super::{RequestActorRole, RequestState};
use crate::{error::DomainError, repository::access::RepositoryAccess, views::Views};

pub const AUTHOR_RECORD_ROLE: RequestActorRole = RequestActorRole::Public;

#[derive(Clone, Copy, Debug)]
pub struct AuthoredRequest<'a> {
    pub author_user_id: Option<&'a str>,
    pub author_role: RequestActorRole,
    pub state: RequestState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorRecordTotals {
    pub merged: u64,
    pub closed_by_maintainer: u64,
    pub withdrawn: u64,
    pub open: u64,
    pub rating_score_sum: u64,
    pub rating_count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorRating {
    pub score_sum: u64,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorHistory {
    pub merged: u64,
    pub closed_by_maintainer: u64,
    pub withdrawn: u64,
    pub other_open: u64,
    pub rating: Option<AuthorRating>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorRecord {
    FirstRequest,
    History(AuthorHistory),
}

pub struct AuthorRecordSubject<'a> {
    pub user_id: &'a str,
    state: RequestState,
}

pub fn author_record_viewer_can_read(viewer: &RepositoryAccess, views: &Views) -> bool {
    viewer.is_maintainer() && viewer.reads_full_view(views)
}

pub fn author_record_subject<'a>(
    request: AuthoredRequest<'a>,
    viewer: &RepositoryAccess,
    views: &Views,
) -> Option<AuthorRecordSubject<'a>> {
    if request.author_role != AUTHOR_RECORD_ROLE || !author_record_viewer_can_read(viewer, views) {
        return None;
    }
    Some(AuthorRecordSubject {
        user_id: request.author_user_id?,
        state: request.state,
    })
}

impl AuthorRecordSubject<'_> {
    pub fn record(self, totals: AuthorRecordTotals) -> Result<AuthorRecord, DomainError> {
        let own_open = u64::from(self.state == RequestState::Open);
        let history = AuthorHistory {
            merged: totals.merged,
            closed_by_maintainer: totals.closed_by_maintainer,
            withdrawn: totals.withdrawn,
            other_open: totals.open.saturating_sub(own_open),
            rating: author_rating(totals.rating_score_sum, totals.rating_count)?,
        };
        let counts = [
            history.merged,
            history.closed_by_maintainer,
            history.withdrawn,
            history.other_open,
        ];
        Ok(
            if counts.into_iter().all(|count| count == 0) && history.rating.is_none() {
                AuthorRecord::FirstRequest
            } else {
                AuthorRecord::History(history)
            },
        )
    }
}

fn author_rating(score_sum: u64, count: u64) -> Result<Option<AuthorRating>, DomainError> {
    if !(count..=count.saturating_mul(5)).contains(&score_sum) {
        return Err(DomainError::invariant_violation(
            "author rating totals are inconsistent",
        ));
    }
    Ok((count > 0).then_some(AuthorRating { score_sum, count }))
}

#[cfg(test)]
mod tests;
