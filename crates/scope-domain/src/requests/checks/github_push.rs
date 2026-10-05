use super::RequestCheckEvaluation;
use crate::github_connection::GitHubConnection;

const REQUEST_BRANCH_PREFIX: &str = "scope/requests/";
const SETUP_CHECK_BRANCH: &str = "scope/setup-check";
const RETRY_DELAYS_SECS: [u64; 4] = [30, 2 * 60, 10 * 60, 30 * 60];
const WORKFLOWS_DIRECTORY: &str = ".github/workflows/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubBranch {
    Request(String),
    SetupCheck,
}

impl GitHubBranch {
    pub fn parse(branch: &str) -> Option<Self> {
        if branch == SETUP_CHECK_BRANCH {
            return Some(Self::SetupCheck);
        }
        branch
            .strip_prefix(REQUEST_BRANCH_PREFIX)
            .filter(|request_id| !request_id.is_empty() && !request_id.contains('/'))
            .map(|request_id| Self::Request(request_id.to_string()))
    }

    pub fn name(&self) -> String {
        match self {
            Self::Request(request_id) => format!("{REQUEST_BRANCH_PREFIX}{request_id}"),
            Self::SetupCheck => SETUP_CHECK_BRANCH.to_string(),
        }
    }

    pub fn git_ref(&self) -> String {
        format!("refs/heads/{}", self.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitHubPushState {
    Queued,
    Running,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPushDestination {
    pub installation_id: u64,
    pub github_repository_id: u64,
    pub github_full_name: String,
}

impl GitHubPushDestination {
    pub fn of(connection: &GitHubConnection) -> Self {
        Self {
            installation_id: connection.installation_id,
            github_repository_id: connection.github_repository_id,
            github_full_name: connection.github_full_name.clone(),
        }
    }

    pub fn is_connected_through(&self, connection: &GitHubConnection) -> bool {
        connection.is_connected()
            && connection.installation_id == self.installation_id
            && connection.github_repository_id == self.github_repository_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubPush {
    pub id: String,
    pub repo_id: String,
    pub branch: GitHubBranch,
    pub target_oid: Option<String>,
    pub destination: GitHubPushDestination,
    pub state: GitHubPushState,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub updated_at_unix: u64,
}

pub fn github_push_retry_at(push: &GitHubPush, now_unix: u64) -> Option<u64> {
    if push.branch == GitHubBranch::SetupCheck && push.target_oid.is_some() {
        return None;
    }
    github_retry_at(push.attempts, now_unix)
}

pub fn github_retry_at(attempts: u32, now_unix: u64) -> Option<u64> {
    let index = usize::try_from(attempts.checked_sub(1)?).ok()?;
    RETRY_DELAYS_SECS
        .get(index)
        .map(|delay| now_unix.saturating_add(*delay))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubPushStatus {
    AwaitingApproval,
    Sending { last_error: Option<String> },
    Sent,
    Failed { error: String },
}

impl GitHubPushStatus {
    pub fn for_evaluation(
        evaluation: &RequestCheckEvaluation,
        push: Option<&GitHubPush>,
    ) -> Option<Self> {
        let push = push.filter(|push| push.target_oid.as_deref() == Some(&evaluation.tested_oid));
        let Some(push) = push else {
            return (evaluation.awaits_approval() && evaluation.asks_github())
                .then_some(Self::AwaitingApproval);
        };
        Some(match push.state {
            GitHubPushState::Queued | GitHubPushState::Running => Self::Sending {
                last_error: push.last_error.clone(),
            },
            GitHubPushState::Succeeded => Self::Sent,
            GitHubPushState::Failed => Self::Failed {
                error: push
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "The push to GitHub failed.".to_string()),
            },
        })
    }
}

pub fn changes_github_workflows<'a>(paths: impl IntoIterator<Item = &'a str>) -> bool {
    paths.into_iter().any(|path| {
        path.trim_start_matches('/')
            .starts_with(WORKFLOWS_DIRECTORY)
    })
}
