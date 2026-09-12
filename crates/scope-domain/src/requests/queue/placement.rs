use super::{RequestAttentionState, RequestQueueFacts, RequestQueueSection};
use crate::repository::access::RepositoryAccess;
use crate::requests::{RequestListPredicate, RequestState, request_list_predicate};

impl RequestQueueSection {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Unclaimed => "unclaimed",
            Self::SetAside => "set_aside",
            Self::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestQueuePredicateAtom {
    Terminal,
    ViewerIsMaintainer,
    ViewerIsNotMaintainer,
    ViewerIsAuthor,
    ViewerIsInvitee,
    AttentionIsWaitingOrSettled,
    AttentionIsActive,
    SnoozedAfterNow,
    SnoozedAtOrBeforeNow,
    ClaimedByViewer,
    ClaimedByOther,
    RequestIsOpen,
    Always,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestQueuePredicate {
    Atom(RequestQueuePredicateAtom),
    All(RequestQueuePredicateAtom, RequestQueuePredicateAtom),
}

impl RequestQueuePredicate {
    pub fn matches(self, facts: &RequestQueueFacts<'_>) -> bool {
        match self {
            Self::Atom(atom) => atom.matches(facts),
            Self::All(left, right) => left.matches(facts) && right.matches(facts),
        }
    }
}

impl RequestQueuePredicateAtom {
    fn matches(self, facts: &RequestQueueFacts<'_>) -> bool {
        match self {
            Self::Terminal => matches!(
                facts.request_state,
                RequestState::Closed | RequestState::Merged
            ),
            Self::ViewerIsMaintainer => facts.viewer_is_maintainer,
            Self::ViewerIsNotMaintainer => !facts.viewer_is_maintainer,
            Self::ViewerIsAuthor => facts
                .viewer_user_id
                .is_some_and(|viewer| viewer == facts.request_author_user_id),
            Self::ViewerIsInvitee => facts.viewer_is_invitee,
            Self::AttentionIsWaitingOrSettled => facts.attention.is_some_and(|attention| {
                matches!(
                    attention.state,
                    RequestAttentionState::Waiting | RequestAttentionState::Settled
                )
            }),
            Self::AttentionIsActive => facts
                .attention
                .is_some_and(|attention| attention.state == RequestAttentionState::Active),
            Self::SnoozedAfterNow => facts.attention.is_some_and(|attention| {
                attention.state == RequestAttentionState::Snoozed
                    && attention
                        .snoozed_until_unix
                        .is_some_and(|until| until > facts.now_unix)
            }),
            Self::SnoozedAtOrBeforeNow => facts.attention.is_some_and(|attention| {
                attention.state == RequestAttentionState::Snoozed
                    && attention
                        .snoozed_until_unix
                        .is_some_and(|until| until <= facts.now_unix)
            }),
            Self::ClaimedByViewer => facts
                .viewer_user_id
                .zip(facts.claim)
                .is_some_and(|(viewer, claim)| claim.claimer_user_id == viewer),
            Self::ClaimedByOther => facts
                .viewer_user_id
                .zip(facts.claim)
                .is_some_and(|(viewer, claim)| claim.claimer_user_id != viewer),
            Self::RequestIsOpen => facts.request_state == RequestState::Open,
            Self::Always => true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestQueueRule {
    Terminal,
    Waiting,
    Snoozed,
    ClaimedElsewhere,
    ActiveAttention,
    SnoozeExpired,
    Claimed,
    Authored,
    Invited,
    Open,
    Unclaimed,
}

pub const REQUEST_QUEUE_RULES: &[RequestQueueRule] = &[
    RequestQueueRule::Terminal,
    RequestQueueRule::Waiting,
    RequestQueueRule::Snoozed,
    RequestQueueRule::ClaimedElsewhere,
    RequestQueueRule::ActiveAttention,
    RequestQueueRule::SnoozeExpired,
    RequestQueueRule::Claimed,
    RequestQueueRule::Authored,
    RequestQueueRule::Invited,
    RequestQueueRule::Open,
    RequestQueueRule::Unclaimed,
];

impl RequestQueueRule {
    pub const fn section(self) -> RequestQueueSection {
        match self {
            Self::Terminal => RequestQueueSection::Done,
            Self::Waiting | Self::Snoozed | Self::ClaimedElsewhere => RequestQueueSection::SetAside,
            Self::ActiveAttention
            | Self::SnoozeExpired
            | Self::Claimed
            | Self::Authored
            | Self::Invited
            | Self::Open => RequestQueueSection::Active,
            Self::Unclaimed => RequestQueueSection::Unclaimed,
        }
    }

    pub const fn predicate(self) -> RequestQueuePredicate {
        use RequestQueuePredicateAtom as Atom;
        match self {
            Self::Terminal => RequestQueuePredicate::Atom(Atom::Terminal),
            Self::Waiting => RequestQueuePredicate::All(
                Atom::ViewerIsMaintainer,
                Atom::AttentionIsWaitingOrSettled,
            ),
            Self::Snoozed => {
                RequestQueuePredicate::All(Atom::ViewerIsMaintainer, Atom::SnoozedAfterNow)
            }
            Self::ClaimedElsewhere => {
                RequestQueuePredicate::All(Atom::ViewerIsMaintainer, Atom::ClaimedByOther)
            }
            Self::ActiveAttention => {
                RequestQueuePredicate::All(Atom::ViewerIsMaintainer, Atom::AttentionIsActive)
            }
            Self::SnoozeExpired => {
                RequestQueuePredicate::All(Atom::ViewerIsMaintainer, Atom::SnoozedAtOrBeforeNow)
            }
            Self::Claimed => {
                RequestQueuePredicate::All(Atom::ViewerIsMaintainer, Atom::ClaimedByViewer)
            }
            Self::Authored => RequestQueuePredicate::Atom(Atom::ViewerIsAuthor),
            Self::Invited => RequestQueuePredicate::Atom(Atom::ViewerIsInvitee),
            Self::Open => {
                RequestQueuePredicate::All(Atom::ViewerIsNotMaintainer, Atom::RequestIsOpen)
            }
            Self::Unclaimed => RequestQueuePredicate::Atom(Atom::Always),
        }
    }
}

pub fn request_queue_visibility_predicate<'a>(
    access: RepositoryAccess,
    viewer_user_id: Option<&'a str>,
) -> RequestListPredicate<'a> {
    let mut participant_or_submitted = vec![RequestListPredicate::Submitted];
    if let Some(viewer_user_id) = viewer_user_id {
        participant_or_submitted.push(RequestListPredicate::Author(viewer_user_id));
        participant_or_submitted.push(RequestListPredicate::Invitee(viewer_user_id));
    }
    RequestListPredicate::All(vec![
        request_list_predicate(access, viewer_user_id),
        RequestListPredicate::Any(participant_or_submitted),
    ])
}
