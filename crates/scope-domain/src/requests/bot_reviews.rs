use crate::{
    github_review_bots::GitHubReviewBot,
    views::{ViewId, Views},
};
use serde::{Deserialize, Serialize};

mod codex_state;
pub use codex_state::{CODEX_REVIEW_BOT_LOGIN, CodexReviewState};

#[cfg(test)]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubReviewBotState {
    NotStarted,
    Working,
    Commented,
    Clear,
}

impl GitHubReviewBotState {
    pub fn is_done(self) -> bool {
        matches!(self, Self::Commented | Self::Clear)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubBotReaction {
    Eyes,
    ThumbsUp,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubReviewBotSignal {
    Reviewed {
        login: String,
        review_id: u64,
        commit_oid: String,
    },
    Reacted {
        login: String,
        reaction: GitHubBotReaction,
        at_unix: u64,
    },
    Commented {
        login: String,
        body: String,
    },
}

impl GitHubReviewBotSignal {
    fn login(&self) -> &str {
        match self {
            Self::Reviewed { login, .. }
            | Self::Reacted { login, .. }
            | Self::Commented { login, .. } => login,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubReviewBotStatus {
    pub login: String,
    pub head_oid: String,
    pub state: GitHubReviewBotState,
}

pub fn github_review_bot_statuses(
    bots: &[GitHubReviewBot],
    signals: &[GitHubReviewBotSignal],
    head_oid: &str,
    head_delivered_at_unix: Option<u64>,
    imported_review_ids: &[u64],
) -> Vec<GitHubReviewBotStatus> {
    bots.iter()
        .map(|bot| {
            let signals = signals
                .iter()
                .filter(|signal| bot.login.names(signal.login()))
                .collect::<Vec<_>>();
            GitHubReviewBotStatus {
                login: bot.login.as_str().to_string(),
                head_oid: head_oid.to_string(),
                state: bot_state(
                    bot.login.names(CODEX_REVIEW_BOT_LOGIN),
                    &signals,
                    head_oid,
                    head_delivered_at_unix,
                    imported_review_ids,
                ),
            }
        })
        .collect()
}

pub fn github_reviews_awaiting_import(
    bots: &[GitHubReviewBot],
    signals: &[GitHubReviewBotSignal],
    head_oid: &str,
    imported_review_ids: &[u64],
) -> Vec<u64> {
    signals
        .iter()
        .filter_map(|signal| match signal {
            GitHubReviewBotSignal::Reviewed {
                login,
                review_id,
                commit_oid,
            } if commit_oid == head_oid
                && !imported_review_ids.contains(review_id)
                && bots.iter().any(|bot| bot.login.names(login)) =>
            {
                Some(*review_id)
            }
            _ => None,
        })
        .collect()
}

fn bot_state(
    reports_state: bool,
    signals: &[&GitHubReviewBotSignal],
    head_oid: &str,
    head_delivered_at_unix: Option<u64>,
    imported_review_ids: &[u64],
) -> GitHubReviewBotState {
    let reacted = |wanted: GitHubBotReaction| {
        signals.iter().any(|signal| {
            matches!(signal, GitHubReviewBotSignal::Reacted { reaction, at_unix, .. }
                if *reaction == wanted
                    && (wanted == GitHubBotReaction::Eyes
                        || head_delivered_at_unix.is_some_and(|delivered| *at_unix > delivered)))
        })
    };
    let working = || {
        if reacted(GitHubBotReaction::Eyes) {
            GitHubReviewBotState::Working
        } else {
            GitHubReviewBotState::NotStarted
        }
    };
    let reported = signals
        .iter()
        .filter_map(|signal| match signal {
            GitHubReviewBotSignal::Commented { body, .. } => CodexReviewState::parse(body),
            _ => None,
        })
        .collect::<Vec<_>>();
    let head_reviews = signals
        .iter()
        .filter_map(|signal| match signal {
            GitHubReviewBotSignal::Reviewed {
                review_id,
                commit_oid,
                ..
            } if commit_oid == head_oid => Some(*review_id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let unimported_review = head_reviews
        .iter()
        .any(|review_id| !imported_review_ids.contains(review_id));
    if reports_state || !reported.is_empty() {
        let states = reported
            .iter()
            .filter_map(|state| state.state_for(head_oid))
            .collect::<Vec<_>>();
        let state = states
            .iter()
            .copied()
            .find(|state| state.is_done())
            .or_else(|| states.first().copied())
            .unwrap_or_else(working);
        let findings_unseen = reported
            .iter()
            .any(|state| state.reports_unresolved_findings_for(head_oid))
            && head_reviews.is_empty();
        return if state.is_done() && (unimported_review || findings_unseen) {
            GitHubReviewBotState::Working
        } else {
            state
        };
    }
    if unimported_review {
        GitHubReviewBotState::Working
    } else if !head_reviews.is_empty() {
        GitHubReviewBotState::Commented
    } else if reacted(GitHubBotReaction::ThumbsUp) {
        GitHubReviewBotState::Clear
    } else {
        working()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestReviewsOutcome {
    Clear,
    Pending,
    FindingsOpen,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RequestBotReviews {
    pub bots: Vec<GitHubReviewBot>,
    pub statuses: Vec<GitHubReviewBotStatus>,
    pub open_finding_authors: Vec<String>,
}

impl RequestBotReviews {
    pub fn state_of(&self, bot: &GitHubReviewBot, head_oid: &str) -> GitHubReviewBotState {
        self.statuses
            .iter()
            .find(|status| status.head_oid == head_oid && bot.login.names(&status.login))
            .map_or(GitHubReviewBotState::NotStarted, |status| status.state)
    }

    fn has_open_findings(&self) -> bool {
        self.open_finding_authors
            .iter()
            .any(|author| self.bots.iter().any(|bot| bot.login.names(author)))
    }
}

pub fn request_reviews_outcome(
    view: &ViewId,
    head_oid: &str,
    views: &Views,
    reviews: &RequestBotReviews,
) -> RequestReviewsOutcome {
    let mut required = reviews.bots.iter().filter(|bot| bot.required).peekable();
    if view != views.full() || required.peek().is_none() {
        RequestReviewsOutcome::Clear
    } else if reviews.has_open_findings() {
        RequestReviewsOutcome::FindingsOpen
    } else if required.any(|bot| !reviews.state_of(bot, head_oid).is_done()) {
        RequestReviewsOutcome::Pending
    } else {
        RequestReviewsOutcome::Clear
    }
}
