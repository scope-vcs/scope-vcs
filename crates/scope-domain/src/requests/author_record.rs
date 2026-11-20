use super::RequestState;
use crate::{error::DomainError, repository::access::RepositoryAccess, views::Views};

#[derive(Clone, Copy, Debug)]
pub struct AuthoredRequest<'a> {
    pub author_user_id: Option<&'a str>,
    pub state: RequestState,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AuthorRecordTotals {
    pub merged: u64,
    pub merged_by_author: u64,
    pub closed: u64,
    pub withdrawn: u64,
    pub open: u64,
    pub rating_score_sum: u64,
    pub rating_count: u64,
}

impl AuthorRecordTotals {
    pub fn count(&mut self, outcome: AuthorOutcome, requests: u64) {
        match outcome {
            AuthorOutcome::Merged => self.merged = requests,
            AuthorOutcome::MergedByAuthor => self.merged_by_author = requests,
            AuthorOutcome::Closed => self.closed = requests,
            AuthorOutcome::Withdrawn => self.withdrawn = requests,
            AuthorOutcome::Open => self.open = requests,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorRequestFact {
    Submitted,
    Merged,
    MergedByAuthor,
    Closed,
    ClosedByAuthor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorRequestCondition {
    pub fact: AuthorRequestFact,
    pub holds: bool,
}

const fn is(fact: AuthorRequestFact) -> AuthorRequestCondition {
    AuthorRequestCondition { fact, holds: true }
}

const fn not(fact: AuthorRequestFact) -> AuthorRequestCondition {
    AuthorRequestCondition { fact, holds: false }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthorOutcome {
    Merged,
    MergedByAuthor,
    Closed,
    Withdrawn,
    Open,
}

pub const AUTHOR_RECORD_COUNTS: [AuthorRequestCondition; 1] = [is(AuthorRequestFact::Submitted)];

pub const AUTHOR_OUTCOME_RULES: [(AuthorOutcome, &[AuthorRequestCondition]); 5] = [
    (
        AuthorOutcome::Merged,
        &[
            is(AuthorRequestFact::Merged),
            not(AuthorRequestFact::MergedByAuthor),
        ],
    ),
    (
        AuthorOutcome::MergedByAuthor,
        &[
            is(AuthorRequestFact::Merged),
            is(AuthorRequestFact::MergedByAuthor),
        ],
    ),
    (
        AuthorOutcome::Closed,
        &[
            is(AuthorRequestFact::Closed),
            not(AuthorRequestFact::ClosedByAuthor),
        ],
    ),
    (
        AuthorOutcome::Withdrawn,
        &[
            is(AuthorRequestFact::Closed),
            is(AuthorRequestFact::ClosedByAuthor),
        ],
    ),
    (
        AuthorOutcome::Open,
        &[
            not(AuthorRequestFact::Merged),
            not(AuthorRequestFact::Closed),
        ],
    ),
];

#[derive(Clone, Copy, Debug)]
pub struct AuthorRequestFacts {
    pub submitted: bool,
    pub merged: bool,
    pub merged_by_author: bool,
    pub closed: bool,
    pub closed_by_author: bool,
}

impl AuthorRequestCondition {
    pub fn matches(self, facts: &AuthorRequestFacts) -> bool {
        let value = match self.fact {
            AuthorRequestFact::Submitted => facts.submitted,
            AuthorRequestFact::Merged => facts.merged,
            AuthorRequestFact::MergedByAuthor => facts.merged_by_author,
            AuthorRequestFact::Closed => facts.closed,
            AuthorRequestFact::ClosedByAuthor => facts.closed_by_author,
        };
        value == self.holds
    }
}

pub fn author_request_outcomes(
    facts: &AuthorRequestFacts,
) -> impl Iterator<Item = AuthorOutcome> + '_ {
    let counted = AUTHOR_RECORD_COUNTS
        .iter()
        .all(|condition| condition.matches(facts));
    AUTHOR_OUTCOME_RULES
        .iter()
        .filter(move |(_, when)| counted && when.iter().all(|condition| condition.matches(facts)))
        .map(|(outcome, _)| *outcome)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorRating {
    pub score_sum: u64,
    pub count: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuthorHistory {
    pub merged: u64,
    pub closed: u64,
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
    views: &'a Views,
}

pub fn author_record_viewer_can_read(viewer: &RepositoryAccess, views: &Views) -> bool {
    viewer.reads_full_view(views)
}

pub fn author_record_subject<'a>(
    request: AuthoredRequest<'a>,
    viewer: &RepositoryAccess,
    views: &'a Views,
) -> Option<AuthorRecordSubject<'a>> {
    if !author_record_viewer_can_read(viewer, views) {
        return None;
    }
    Some(AuthorRecordSubject {
        user_id: request.author_user_id?,
        state: request.state,
        views,
    })
}

impl AuthorRecordSubject<'_> {
    pub fn record(
        self,
        author: &RepositoryAccess,
        totals: AuthorRecordTotals,
    ) -> Result<Option<AuthorRecord>, DomainError> {
        if author.reads_full_view(self.views) {
            return Ok(None);
        }
        let own_open = u64::from(self.state == RequestState::Open);
        let history = AuthorHistory {
            merged: totals.merged,
            closed: totals.closed,
            withdrawn: totals.withdrawn,
            other_open: totals.open.saturating_sub(own_open),
            rating: author_rating(totals.rating_score_sum, totals.rating_count)?,
        };
        let other_requests = [
            history.merged,
            totals.merged_by_author,
            history.closed,
            history.withdrawn,
            history.other_open,
        ];
        Ok(Some(
            if other_requests.into_iter().all(|count| count == 0) && history.rating.is_none() {
                AuthorRecord::FirstRequest
            } else {
                AuthorRecord::History(history)
            },
        ))
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
