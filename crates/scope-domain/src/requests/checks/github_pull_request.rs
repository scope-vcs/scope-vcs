use super::{GitHubBranch, GitHubCheckTarget, GitHubPushDestination};
use crate::{
    github_connection::{GitHubConnection, GitHubReviewBotSettings},
    requests::{Request, RequestState},
    views::Views,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MAIN_BRANCH: &str = "main";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequestMerge {
    pub merge_commit_oid: String,
    pub head_oid: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestState {
    Open,
    Closed,
    ClosedOnGitHub,
    Merged(GitHubPullRequestMerge),
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
pub struct GitHubReviewRequest {
    pub head_oid: String,
    pub at_unix: u64,
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
    pub review_request: Option<GitHubReviewRequest>,
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
    pub state: GitHubPullRequestObservedState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestObservedState {
    Open,
    Merged(GitHubPullRequestMerge),
    Closed { by_scope: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestStatusState {
    Opening,
    Open,
    Closed,
    ClosedOnGitHub,
    Merged,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequestStatus {
    pub state: GitHubPullRequestStatusState,
    pub number: Option<u64>,
    pub url: Option<String>,
    pub error: Option<String>,
}

pub fn carrier_pull_request_base(
    connection: &GitHubConnection,
    request: &Request,
    parent: Option<&Request>,
    parent_carrier: Option<&GitHubPullRequest>,
    views: &Views,
) -> Option<String> {
    let wanted = connection.is_connected()
        && connection.review_bots.open_pull_requests
        && request.state() == RequestState::Open
        && GitHubCheckTarget::for_request(request, views) == GitHubCheckTarget::Head
        && may_carry_on_github(connection, request);
    wanted.then(|| parent_branch_or_main(connection, request, parent, parent_carrier))
}

pub fn may_carry_on_github(connection: &GitHubConnection, request: &Request) -> bool {
    !carries_private_request(request) || connection.may_receive_private_requests()
}

pub fn carries_private_request(request: &Request) -> bool {
    !request.view.is_public()
}

fn parent_branch_or_main(
    connection: &GitHubConnection,
    request: &Request,
    parent: Option<&Request>,
    parent_carrier: Option<&GitHubPullRequest>,
) -> String {
    let parent_ended_on_github = parent_carrier.is_some_and(|carrier| {
        carrier.destination.is_connected_through(connection) && carrier.has_ended_on_github()
    });
    request
        .base
        .parent_request_id()
        .and_then(|parent_id| {
            parent.filter(|parent| {
                parent.id == parent_id
                    && parent.state() == RequestState::Open
                    && !parent_ended_on_github
            })
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

pub fn closes_carrier_pull_request(
    request: &Request,
    connection: Option<&GitHubConnection>,
) -> bool {
    match request.state() {
        RequestState::Closed => true,
        RequestState::Merged => !connection.is_some_and(|connection| {
            GitHubPushDestination::for_main_publication(connection).is_some()
        }),
        RequestState::Draft | RequestState::Open => false,
    }
}

pub fn retains_request_branch_for_carrier(
    request: &Request,
    connection: Option<&GitHubConnection>,
    carrier: Option<&GitHubPullRequest>,
) -> bool {
    !request.is_terminal()
        || (!closes_carrier_pull_request(request, connection)
            && carrier.is_some_and(GitHubPullRequest::is_open_on_github))
}

pub(super) const BRANCH_IS_A_PULL_REQUEST_BASE_MESSAGE: &str = "An open pull request from a Scope request \
    is still based on this branch. Scope deletes the branch once that pull request moves or closes.";

pub(super) const BRANCH_HAS_UNLISTED_PULL_REQUESTS_MESSAGE: &str = "More open pull requests are based on \
    this branch than Scope reads from GitHub. Scope deletes the branch once it can read them all \
    and none comes from a Scope request.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPullRequestHead {
    pub github_repository_id: Option<u64>,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPullRequestsBasedOnBranch {
    Unreadable,
    Open(Vec<GitHubPullRequestHead>),
    OpenBeyondListed(Vec<GitHubPullRequestHead>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubBranchDeletion {
    Delete,
    Wait {
        requests: Vec<String>,
        reason: &'static str,
    },
}

impl GitHubPullRequestsBasedOnBranch {
    pub fn branch_deletion(&self, destination: &GitHubPushDestination) -> GitHubBranchDeletion {
        let (heads, all_listed) = match self {
            Self::Unreadable => return GitHubBranchDeletion::Delete,
            Self::Open(heads) => (heads, true),
            Self::OpenBeyondListed(heads) => (heads, false),
        };
        let requests = heads
            .iter()
            .filter(|head| head.github_repository_id == Some(destination.github_repository_id))
            .filter_map(|head| match GitHubBranch::parse(&head.branch) {
                Some(GitHubBranch::Request(request_id)) => Some(request_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        match (requests.is_empty(), all_listed) {
            (true, true) => GitHubBranchDeletion::Delete,
            (true, false) => GitHubBranchDeletion::Wait {
                requests,
                reason: BRANCH_HAS_UNLISTED_PULL_REQUESTS_MESSAGE,
            },
            (false, _) => GitHubBranchDeletion::Wait {
                requests,
                reason: BRANCH_IS_A_PULL_REQUEST_BASE_MESSAGE,
            },
        }
    }
}

pub fn review_delivery_belongs_to_request(
    carrier: Option<&GitHubPullRequest>,
    pull_number: u64,
) -> bool {
    carrier
        .and_then(|carrier| carrier.pull_request.as_ref())
        .is_none_or(|opened| opened.number == pull_number)
}

pub fn carrier_may_change(request: &Request) -> bool {
    request.state() == RequestState::Open
}

impl GitHubPullRequest {
    pub fn branch(&self) -> GitHubBranch {
        GitHubBranch::Request(self.request_id.clone())
    }

    pub fn has_ended_on_github(&self) -> bool {
        self.pull_request.is_some() && !self.is_open_on_github()
    }

    pub fn is_open_on_github(&self) -> bool {
        self.pull_request
            .as_ref()
            .is_some_and(|opened| opened.state == GitHubPullRequestState::Open)
    }

    pub fn merge_of(&self, request: &Request) -> Option<&GitHubPullRequestMerge> {
        match &self.pull_request.as_ref()?.state {
            GitHubPullRequestState::Merged(merge) if merge.head_oid == request.head_oid => {
                Some(merge)
            }
            _ => None,
        }
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
                Some(opened)
                    if matches!(
                        opened.state,
                        GitHubPullRequestState::ClosedOnGitHub | GitHubPullRequestState::Merged(_)
                    ) =>
                {
                    None
                }
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
                        && self
                            .review_request
                            .as_ref()
                            .is_none_or(|request| &request.head_oid != head_oid)
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
            self.review_request = None;
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

    pub fn record_commented(&mut self, head_oid: String, at_unix: u64) {
        self.review_request = Some(GitHubReviewRequest { head_oid, at_unix });
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
        opened.state = match observation.state {
            GitHubPullRequestObservedState::Open => GitHubPullRequestState::Open,
            GitHubPullRequestObservedState::Merged(merge) => GitHubPullRequestState::Merged(merge),
            GitHubPullRequestObservedState::Closed { by_scope } if by_scope || moving_base => {
                GitHubPullRequestState::Closed
            }
            GitHubPullRequestObservedState::Closed { .. } => GitHubPullRequestState::ClosedOnGitHub,
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
                GitHubPullRequestState::Merged(_) => GitHubPullRequestStatusState::Merged,
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
