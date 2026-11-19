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

pub fn author_record_subject<'a>(
    request: AuthoredRequest<'a>,
    viewer: &RepositoryAccess,
    views: &Views,
) -> Option<&'a str> {
    let viewer_reads_every_request = viewer.is_maintainer() && viewer.reads_full_view(views);
    (viewer_reads_every_request && request.author_role == AUTHOR_RECORD_ROLE)
        .then_some(request.author_user_id)
        .flatten()
}

pub fn author_record(
    request: AuthoredRequest<'_>,
    viewer: &RepositoryAccess,
    views: &Views,
    totals: AuthorRecordTotals,
) -> Result<Option<AuthorRecord>, DomainError> {
    if author_record_subject(request, viewer, views).is_none() {
        return Ok(None);
    }
    let own_open = u64::from(request.state == RequestState::Open);
    let history = AuthorHistory {
        merged: totals.merged,
        closed_by_maintainer: totals.closed_by_maintainer,
        withdrawn: totals.withdrawn,
        other_open: totals.open.saturating_sub(own_open),
        rating: author_rating(totals.rating_score_sum, totals.rating_count)?,
    };
    let other_requests =
        history.merged + history.closed_by_maintainer + history.withdrawn + history.other_open;
    Ok(Some(if other_requests == 0 && history.rating.is_none() {
        AuthorRecord::FirstRequest
    } else {
        AuthorRecord::History(history)
    }))
}

fn author_rating(score_sum: u64, count: u64) -> Result<Option<AuthorRating>, DomainError> {
    if count == 0 {
        return if score_sum == 0 {
            Ok(None)
        } else {
            Err(DomainError::invariant_violation(
                "author rating totals are inconsistent",
            ))
        };
    }
    let maximum = count
        .checked_mul(5)
        .ok_or_else(|| DomainError::invariant_violation("author rating count overflow"))?;
    if !(count..=maximum).contains(&score_sum) {
        return Err(DomainError::invariant_violation(
            "author rating totals are inconsistent",
        ));
    }
    Ok(Some(AuthorRating { score_sum, count }))
}

#[cfg(test)]
mod tests;
