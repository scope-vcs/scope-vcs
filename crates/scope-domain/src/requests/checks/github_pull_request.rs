use super::{GitHubBranch, GitHubCheckTarget, GitHubPushDestination};
use crate::{
    github_connection::{GitHubConnection, GitHubReviewBotSettings},
    requests::{Request, RequestState},
    views::Views,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAIN_BRANCH: &str = "main";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubPullRequestState {
    Open,
    Closed,
    #[serde(rename = "closed_on_github")]
    ClosedOnGitHub,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenedGitHubPullRequest {
    pub number: u64,
    pub url: String,
    pub base: String,
    pub state: GitHubPullRequestState,
    pub written_text: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestIntent {
    Open { base: String, head_oid: String },
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubPullRequestJobState {
    Queued,
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequest {
    pub request_id: String,
    pub repo_id: String,
    pub destination: GitHubPushDestination,
    pub intent: GitHubPullRequestIntent,
    pub intent_sequence: u64,
    pub pull_request: Option<OpenedGitHubPullRequest>,
    pub commented_head_oid: Option<String>,
    pub job_state: GitHubPullRequestJobState,
    pub attempts: u32,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestStep {
    OpenOrAdopt { base: String },
    Retarget { number: u64, base: String },
    UpdateText { number: u64, text: String },
    Comment { number: u64, head_oid: String },
    AdoptToClose,
    Close { number: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequestObservation {
    pub number: u64,
    pub url: String,
    pub base: String,
    pub open: bool,
    pub closed_by_scope: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestStatusState {
    Opening,
    Open,
    Closed,
    ClosedOnGitHub,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequestStatus {
    pub state: GitHubPullRequestStatusState,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub error: Option<String>,
}

pub fn carrier_pull_request_intent(
    connection: &GitHubConnection,
    request: &Request,
    parent: Option<&Request>,
    views: &Views,
    pushed_head_oid: &str,
) -> Option<GitHubPullRequestIntent> {
    let wanted = connection.is_connected()
        && connection.review_bots.open_pull_requests
        && request.state() == RequestState::Open
        && GitHubCheckTarget::for_request(request, views) == GitHubCheckTarget::Head
        && may_carry_on_github(connection, request);
    wanted.then(|| GitHubPullRequestIntent::Open {
        base: carrier_pull_request_base(request, parent),
        head_oid: pushed_head_oid.to_string(),
    })
}

pub fn may_carry_on_github(connection: &GitHubConnection, request: &Request) -> bool {
    !carries_private_request(request) || connection.may_receive_private_requests()
}

pub fn carries_private_request(request: &Request) -> bool {
    !request.view.is_public()
}

fn carrier_pull_request_base(request: &Request, parent: Option<&Request>) -> String {
    request
        .base
        .parent_request_id()
        .and_then(|parent_id| {
            parent.filter(|parent| parent.id == parent_id && parent.state() == RequestState::Open)
        })
        .map_or_else(
            || MAIN_BRANCH.to_string(),
            |parent| GitHubBranch::Request(parent.id.clone()).name(),
        )
}

pub fn review_request_marker(head_oid: &str) -> String {
    format!("<!-- scope-review-request head={head_oid} -->")
}

pub fn review_request_comment_body(comment: &str, head_oid: &str) -> String {
    format!("{comment}\n\n{}", review_request_marker(head_oid))
}

pub fn carrier_text_digest(title: &str, body: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(title.as_bytes());
    digest.update([0]);
    digest.update(body.as_bytes());
    hex::encode(digest.finalize())
}

pub fn closes_carrier_pull_request(request: &Request) -> bool {
    matches!(request.state(), RequestState::Closed | RequestState::Merged)
}

impl GitHubPullRequest {
    pub fn branch(&self) -> GitHubBranch {
        GitHubBranch::Request(self.request_id.clone())
    }

    pub fn next_step(
        &self,
        review_bots: &GitHubReviewBotSettings,
        text: Option<&str>,
    ) -> Option<GitHubPullRequestStep> {
        let opened = self.pull_request.as_ref();
        match &self.intent {
            GitHubPullRequestIntent::Open { .. } if !review_bots.open_pull_requests => None,
            GitHubPullRequestIntent::Open { base, head_oid } => match opened {
                Some(opened) if opened.state == GitHubPullRequestState::ClosedOnGitHub => None,
                Some(opened) if opened.state == GitHubPullRequestState::Open => {
                    let stale_text = text.filter(|text| {
                        opened
                            .written_text
                            .as_deref()
                            .is_some_and(|written| written != *text)
                    });
                    if &opened.base != base {
                        Some(GitHubPullRequestStep::Retarget {
                            number: opened.number,
                            base: base.clone(),
                        })
                    } else if let Some(text) = stale_text {
                        Some(GitHubPullRequestStep::UpdateText {
                            number: opened.number,
                            text: text.to_string(),
                        })
                    } else if review_bots.review_request_comment.is_some()
                        && self.commented_head_oid.as_ref() != Some(head_oid)
                    {
                        Some(GitHubPullRequestStep::Comment {
                            number: opened.number,
                            head_oid: head_oid.clone(),
                        })
                    } else {
                        None
                    }
                }
                _ => Some(GitHubPullRequestStep::OpenOrAdopt { base: base.clone() }),
            },
            GitHubPullRequestIntent::Close => match opened {
                None => Some(GitHubPullRequestStep::AdoptToClose),
                Some(opened) => (opened.state == GitHubPullRequestState::Open).then_some(
                    GitHubPullRequestStep::Close {
                        number: opened.number,
                    },
                ),
            },
        }
    }

    pub fn record_opened(
        &mut self,
        number: u64,
        url: String,
        base: String,
        written_text: Option<String>,
    ) {
        if self.pull_request.as_ref().map(|opened| opened.number) != Some(number) {
            self.commented_head_oid = None;
        }
        self.pull_request = Some(OpenedGitHubPullRequest {
            number,
            url,
            base,
            state: GitHubPullRequestState::Open,
            written_text,
        });
    }

    pub fn record_text_written(&mut self, text: String) {
        if let Some(opened) = &mut self.pull_request {
            opened.written_text = Some(text);
        }
    }

    pub fn record_retargeted(&mut self, base: String) {
        if let Some(opened) = &mut self.pull_request {
            opened.base = base;
        }
    }

    pub fn record_commented(&mut self, head_oid: String) {
        self.commented_head_oid = Some(head_oid);
    }

    pub fn record_closed(&mut self) {
        if let Some(opened) = &mut self.pull_request {
            opened.state = GitHubPullRequestState::Closed;
        }
    }

    pub fn observe(&mut self, observation: GitHubPullRequestObservation) {
        let moving_base = match &self.intent {
            GitHubPullRequestIntent::Close => true,
            GitHubPullRequestIntent::Open { base, .. } => base != &observation.base,
        };
        let Some(opened) = self
            .pull_request
            .as_mut()
            .filter(|opened| opened.number == observation.number)
        else {
            return;
        };
        opened.url = observation.url;
        opened.base = observation.base;
        opened.state = if observation.open {
            GitHubPullRequestState::Open
        } else if observation.closed_by_scope || moving_base {
            GitHubPullRequestState::Closed
        } else {
            GitHubPullRequestState::ClosedOnGitHub
        };
    }

    pub fn status(&self) -> Option<GitHubPullRequestStatus> {
        let opened = self.pull_request.as_ref();
        let state = match (self.job_state, opened) {
            (GitHubPullRequestJobState::Failed, _) => GitHubPullRequestStatusState::Failed,
            (_, Some(opened)) => match opened.state {
                GitHubPullRequestState::Open => GitHubPullRequestStatusState::Open,
                GitHubPullRequestState::Closed => GitHubPullRequestStatusState::Closed,
                GitHubPullRequestState::ClosedOnGitHub => {
                    GitHubPullRequestStatusState::ClosedOnGitHub
                }
            },
            (GitHubPullRequestJobState::Done, None) => return None,
            (_, None) if self.intent == GitHubPullRequestIntent::Close => return None,
            (_, None) => GitHubPullRequestStatusState::Opening,
        };
        Some(GitHubPullRequestStatus {
            state,
            number: opened.map(|opened| opened.number),
            url: opened.map(|opened| opened.url.clone()),
            error: self.last_error.clone(),
        })
    }
}
