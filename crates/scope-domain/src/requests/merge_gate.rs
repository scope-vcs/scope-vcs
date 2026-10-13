pub const GITHUB_CHECK_MERGE_FRESHNESS_SECS: u64 = 60;

use super::access::request_merger;
use super::{Request, RequestChecksOutcome, RequestState, RequestViewer};
use crate::error::DomainError;
use crate::repository::{
    RepoLifecycleState,
    access::{MainPushMode, RepositoryAccess},
};
use crate::views::{ViewId, Views};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RequestMergeabilityStatus {
    Ready,
    RepositoryNotReady,
    Draft,
    Closed,
    Merged,
    NotMaintainer,
    MissingRequestBranch,
    ChecksNotEvaluated,
    ChecksAwaitingApproval,
    ChecksPending,
    ChecksFailed,
    ChecksConfigurationError,
    WaitingForParent,
    ParentRewritten,
    ParentClosed,
    AncestorBlocked,
    IntegrationConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestMergeability {
    pub status: RequestMergeabilityStatus,
    pub reason: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeGateAuthority {
    RequestMerge,
    AuthorizedMainPush,
}

pub enum MergeGateInput<'a> {
    RequestMerge {
        repository_lifecycle: RepoLifecycleState,
        subject: RequestMergeSubject<'a>,
        viewer: &'a RequestViewer<'a>,
        views: &'a Views,
        checks: RequestChecksOutcome,
    },
    AuthorizedMainPush {
        repository_lifecycle: RepoLifecycleState,
        access: &'a RepositoryAccess,
        views: &'a Views,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeGateDecision {
    RequestMerge(RequestMergeability),
    AuthorizedMainPush,
    MainPushDenied,
}

impl MergeGateDecision {
    pub fn authority(self) -> Option<MergeGateAuthority> {
        match self {
            Self::RequestMerge(decision) if decision.status == RequestMergeabilityStatus::Ready => {
                Some(MergeGateAuthority::RequestMerge)
            }
            Self::AuthorizedMainPush => Some(MergeGateAuthority::AuthorizedMainPush),
            _ => None,
        }
    }

    pub fn reason(self) -> Option<&'static str> {
        match self {
            Self::RequestMerge(decision) => decision.reason,
            Self::AuthorizedMainPush => None,
            Self::MainPushDenied => Some("full-view main push access required"),
        }
    }

    pub fn request_mergeability(self) -> Option<RequestMergeability> {
        match self {
            Self::RequestMerge(decision) => Some(decision),
            _ => None,
        }
    }
}

pub fn evaluate_merge_gate(input: MergeGateInput<'_>) -> MergeGateDecision {
    match input {
        MergeGateInput::RequestMerge {
            repository_lifecycle,
            subject,
            viewer,
            views,
            checks,
        } => MergeGateDecision::RequestMerge(evaluate_request_merge(
            repository_lifecycle,
            subject,
            viewer,
            views,
            checks,
        )),
        MergeGateInput::AuthorizedMainPush {
            repository_lifecycle,
            access,
            views,
        } => match access.main_push_mode(repository_lifecycle, views) {
            MainPushMode::FirstPush | MainPushMode::Ready => MergeGateDecision::AuthorizedMainPush,
            _ => MergeGateDecision::MainPushDenied,
        },
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RequestMergeSubject<'a> {
    pub author_user_id: Option<&'a str>,
    pub view: &'a ViewId,
    pub state: RequestState,
    pub has_git_snapshot: bool,
    pub stack_status: super::RequestStackStatus,
}

impl<'a> From<&'a Request> for RequestMergeSubject<'a> {
    fn from(request: &'a Request) -> Self {
        Self {
            author_user_id: request.author_user_id.as_deref(),
            view: &request.view,
            state: request.state(),
            has_git_snapshot: request.git_snapshot.is_some(),
            stack_status: request.stack_status,
        }
    }
}

pub(super) fn evaluate_request_merge(
    repository_lifecycle: RepoLifecycleState,
    subject: RequestMergeSubject<'_>,
    viewer: &RequestViewer<'_>,
    views: &Views,
    checks: RequestChecksOutcome,
) -> RequestMergeability {
    if repository_lifecycle != RepoLifecycleState::Ready {
        return RequestMergeability {
            status: RequestMergeabilityStatus::RepositoryNotReady,
            reason: Some("repo must be ready before merge"),
        };
    }
    let (status, reason) = match subject.state {
        RequestState::Closed => (RequestMergeabilityStatus::Closed, Some("request is closed")),
        RequestState::Merged => (RequestMergeabilityStatus::Merged, Some("request is merged")),
        RequestState::Draft => (
            RequestMergeabilityStatus::Draft,
            Some("request is not submitted"),
        ),
        RequestState::Open if !viewer.access.is_maintainer() => (
            RequestMergeabilityStatus::NotMaintainer,
            Some("repo maintainer required"),
        ),
        RequestState::Open
            if !request_merger(viewer, subject.author_user_id, subject.view, views) =>
        {
            (
                RequestMergeabilityStatus::NotMaintainer,
                Some("merging needs a maintainer who reads the full view"),
            )
        }
        RequestState::Open if !subject.has_git_snapshot => (
            RequestMergeabilityStatus::MissingRequestBranch,
            Some("request branch has not been pushed"),
        ),
        RequestState::Open if subject.stack_status.requires_restack() => {
            return request_stack_mergeability(subject.stack_status).unwrap();
        }
        RequestState::Open
            if subject.stack_status != super::RequestStackStatus::Main
                && !matches!(
                    checks,
                    RequestChecksOutcome::Failed | RequestChecksOutcome::ConfigurationError
                ) =>
        {
            return request_stack_mergeability(subject.stack_status).unwrap();
        }
        RequestState::Open => match checks {
            RequestChecksOutcome::Clear => (RequestMergeabilityStatus::Ready, None),
            RequestChecksOutcome::NotEvaluated => (
                RequestMergeabilityStatus::ChecksNotEvaluated,
                Some("CI has not been evaluated for this commit yet"),
            ),
            RequestChecksOutcome::AwaitingApproval => (
                RequestMergeabilityStatus::ChecksAwaitingApproval,
                Some("CI is waiting for a maintainer to start it"),
            ),
            RequestChecksOutcome::Pending => (
                RequestMergeabilityStatus::ChecksPending,
                Some("CI has not finished"),
            ),
            RequestChecksOutcome::Failed => (
                RequestMergeabilityStatus::ChecksFailed,
                Some("a required result did not succeed"),
            ),
            RequestChecksOutcome::ConfigurationError => (
                RequestMergeabilityStatus::ChecksConfigurationError,
                Some("the request head's CI cannot run as configured"),
            ),
        },
    };
    RequestMergeability { status, reason }
}

fn request_stack_mergeability(status: super::RequestStackStatus) -> Option<RequestMergeability> {
    let mergeability = match status {
        super::RequestStackStatus::Main => return None,
        super::RequestStackStatus::WaitingForParent | super::RequestStackStatus::ParentExtended => {
            RequestMergeabilityStatus::WaitingForParent
        }
        super::RequestStackStatus::ParentRewritten => RequestMergeabilityStatus::ParentRewritten,
        super::RequestStackStatus::ParentClosed => RequestMergeabilityStatus::ParentClosed,
        super::RequestStackStatus::AncestorBlocked => RequestMergeabilityStatus::AncestorBlocked,
        super::RequestStackStatus::IntegrationConflict => {
            RequestMergeabilityStatus::IntegrationConflict
        }
    };
    Some(RequestMergeability {
        status: mergeability,
        reason: status.blocking_reason(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestMainPushCompletion {
    Complete,
    DeferUntilParentLands,
}

pub fn request_main_push_completion(
    request: &Request,
    contains_request_head: bool,
) -> Result<RequestMainPushCompletion, DomainError> {
    validate_containing_main_head(contains_request_head)?;
    if matches!(request.base, super::RequestBase::Request { .. }) {
        return Ok(RequestMainPushCompletion::DeferUntilParentLands);
    }
    validate_request_merge_execution(
        request,
        super::MergeRequestLanding::ContainingMain {
            contains_request_head,
        },
    )?;
    Ok(RequestMainPushCompletion::Complete)
}

fn validate_containing_main_head(contains_request_head: bool) -> Result<(), DomainError> {
    if !contains_request_head {
        return Err(DomainError::conflict(
            "containing main must contain the request head",
        ));
    }
    Ok(())
}

pub(super) fn validate_request_merge_execution(
    request: &Request,
    landing: super::MergeRequestLanding,
) -> Result<(), DomainError> {
    if let super::MergeRequestLanding::ContainingMain {
        contains_request_head,
    } = landing
    {
        validate_containing_main_head(contains_request_head)?;
        if !matches!(request.base, super::RequestBase::Main { .. }) {
            return Err(DomainError::conflict("parent request must land first"));
        }
        if request.stack_status == super::RequestStackStatus::IntegrationConflict {
            return Ok(());
        }
    }
    if let Some(decision) = request_stack_mergeability(request.stack_status) {
        return Err(DomainError::conflict(decision.reason.unwrap()));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
