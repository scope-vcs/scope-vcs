use crate::{
    repo_events::{
        REQUEST_SUMMARY_REFRESH_VERSION, RepoChangeEvent, RepoChangeKind, RepoChangeReason,
        repository_change_event, request_timeline_change_event,
    },
    state::AppState,
};
use scope_domain::{repository::RepositoryIncarnation, requests::Request, views::ViewId};
use scope_product_analytics::ProductEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Wakeup {
    AutoMerge,
    ClerkUserDeletion,
    GitHubJobLogs,
    GitHubJobReads,
    GitHubPullRequests,
    GitHubPushes,
    GitHubReviewBotStatuses,
    GitHubIssueImports,
    GitHubReviewImports,
    GitHubRunImports,
    GitHubWebhookDeliveries,
    InviteEmails,
    RequestDiffStats,
}

pub(crate) enum AfterCommit {
    Live {
        event: RepoChangeEvent,
        description: &'static str,
    },
    Analytics(ProductEvent),
    Wake(Wakeup),
}

impl AfterCommit {
    pub(crate) fn repo_change(
        incarnation: &RepositoryIncarnation,
        version: u64,
        reason: RepoChangeReason,
    ) -> Self {
        Self::Live {
            event: repository_change_event(incarnation, version, reason),
            description: "repo change",
        }
    }

    pub(crate) fn request_summary(
        incarnation: &RepositoryIncarnation,
        reason: RepoChangeReason,
    ) -> Self {
        Self::repo_change(incarnation, REQUEST_SUMMARY_REFRESH_VERSION, reason)
    }

    pub(crate) fn request_state(incarnation: &RepositoryIncarnation, request: &Request) -> Self {
        Self::Live {
            event: RepoChangeEvent {
                repo_id: incarnation.repository_id().to_string(),
                incarnation_id: incarnation.incarnation_id().to_string(),
                version: REQUEST_SUMMARY_REFRESH_VERSION,
                kind: RepoChangeKind::RequestStateChanged {
                    request_id: request.id.clone(),
                    view: request.view.clone().into(),
                },
            },
            description: "request state",
        }
    }

    pub(crate) fn request_timeline(
        incarnation: &RepositoryIncarnation,
        request_id: String,
        discussion_id: String,
        through_position: u64,
        view: ViewId,
    ) -> Self {
        Self::Live {
            event: request_timeline_change_event(
                incarnation,
                request_id,
                discussion_id,
                through_position,
                view,
            ),
            description: "request discussion",
        }
    }

    pub(crate) fn dependencies_changed(incarnation: &RepositoryIncarnation, version: u64) -> Self {
        Self::Live {
            event: RepoChangeEvent {
                repo_id: incarnation.repository_id().to_string(),
                incarnation_id: incarnation.incarnation_id().to_string(),
                version,
                kind: RepoChangeKind::DependenciesChanged,
            },
            description: "dependency report change",
        }
    }

    pub(crate) fn github_workflow_runs(incarnation: &RepositoryIncarnation) -> Self {
        Self::Live {
            event: RepoChangeEvent {
                repo_id: incarnation.repository_id().to_string(),
                incarnation_id: incarnation.incarnation_id().to_string(),
                version: 0,
                kind: RepoChangeKind::GitHubWorkflowRunsChanged,
            },
            description: "GitHub workflow run",
        }
    }

    pub(crate) fn github_workflow_run(
        incarnation: &RepositoryIncarnation,
        github_run_id: u64,
    ) -> Self {
        Self::Live {
            event: RepoChangeEvent {
                repo_id: incarnation.repository_id().to_string(),
                incarnation_id: incarnation.incarnation_id().to_string(),
                version: 0,
                kind: RepoChangeKind::GitHubWorkflowRunChanged { github_run_id },
            },
            description: "GitHub workflow job",
        }
    }

    pub(crate) fn github_workflow_runs_expired(incarnation: &RepositoryIncarnation) -> Self {
        Self::Live {
            event: RepoChangeEvent {
                repo_id: incarnation.repository_id().to_string(),
                incarnation_id: incarnation.incarnation_id().to_string(),
                version: 0,
                kind: RepoChangeKind::GitHubWorkflowRunsExpired,
            },
            description: "GitHub workflow run retention",
        }
    }
}

impl AppState {
    pub(crate) fn after_commit(&self, effects: impl IntoIterator<Item = AfterCommit>) {
        for effect in effects {
            match effect {
                AfterCommit::Live { event, description } => self.publish_live(event, description),
                AfterCommit::Analytics(event) => self.product_analytics.capture(event),
                AfterCommit::Wake(wakeup) => self.wake(wakeup),
            }
        }
    }

    pub(crate) async fn after_commit_request_state(
        &self,
        incarnation: &RepositoryIncarnation,
        request_id: &str,
    ) {
        let request = match self.metadata.requests().request_by_id(request_id).await {
            Ok(Some(request)) if request.repo_id == incarnation.repository_id() => request,
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(request_id, error = %error, "reading request for state notification failed");
                return;
            }
        };
        self.after_commit([AfterCommit::request_state(incarnation, &request)]);
    }

    fn wake(&self, wakeup: Wakeup) {
        let wakeups = &self.wakeups;
        match wakeup {
            Wakeup::AutoMerge => &wakeups.auto_merge,
            Wakeup::ClerkUserDeletion => &wakeups.clerk_user_deletion,
            Wakeup::GitHubJobLogs => &wakeups.github_job_logs,
            Wakeup::GitHubJobReads => &wakeups.github_job_reads,
            Wakeup::GitHubPullRequests => &wakeups.github_pull_requests,
            Wakeup::GitHubPushes => &wakeups.github_pushes,
            Wakeup::GitHubReviewBotStatuses => &wakeups.github_review_bot_statuses,
            Wakeup::GitHubIssueImports => &wakeups.github_issue_imports,
            Wakeup::GitHubReviewImports => &wakeups.github_review_imports,
            Wakeup::GitHubRunImports => &wakeups.github_run_imports,
            Wakeup::GitHubWebhookDeliveries => &wakeups.github_webhook_deliveries,
            Wakeup::InviteEmails => &wakeups.invite_emails,
            Wakeup::RequestDiffStats => &wakeups.request_diff_stats,
        }
        .notify_one();
    }

    fn publish_live(&self, event: RepoChangeEvent, description: &'static str) {
        self.repo_events.publish_event(event.clone());
        let payload = match self.repo_events.notification_payload(&event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(repo_id = %event.repo_id, %error, "failed to serialize {description} notification");
                return;
            }
        };
        if let Err(error) = self
            .metadata
            .repositories()
            .queue_repo_change_notification(payload)
        {
            tracing::warn!(
                repo_id = %event.repo_id,
                error = %error.message,
                "failed to publish {description} notification"
            );
        }
    }
}
