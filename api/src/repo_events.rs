use scope_api_contract::RepoChangeNotification;
pub(crate) use scope_api_contract::{RepoChangeEvent, RepoChangeKind};
use scope_domain::repository::RepositoryIncarnation;
use scope_domain::views::ViewId;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

const REPO_CHANGE_CHANNEL_CAPACITY: usize = 128;
pub(crate) const REQUEST_SUMMARY_REFRESH_VERSION: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoChangeReason {
    Connected,
    Redacted,
    Lagged,
    RepoDeleted,
    ConfigApplied,
    MetadataUpdated,
    RequestMerged,
    RequestDeleted,
    RequestClosed,
    RequestStarted,
    RequestIdentityEdited,
    RequestInviteeLeft,
    RequestRevised,
    RequestChecksUpdated,
    RequestStackUpdated,
    MemberAdded,
    InviteUpdated,
    MemberPermissionsChanged,
    InviteRevoked,
    MemberRemoved,
    ContributorDeleted,
    FirstPushApplied,
    PushReceived,
    RequestSubmitted,
    RequestRated,
    RequestInviteeAdded,
    RequestInviteeRemoved,
    RequestAttentionChanged,
    GitHubConnectionChanged,
}

impl RepoChangeReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Redacted => "repo-changed",
            Self::Lagged => "lagged",
            Self::RepoDeleted => "repo-deleted",
            Self::ConfigApplied => "config-applied",
            Self::MetadataUpdated => "metadata-updated",
            Self::RequestMerged => "request-merged",
            Self::RequestDeleted => "request-deleted",
            Self::RequestClosed => "request-closed",
            Self::RequestStarted => "request-started",
            Self::RequestIdentityEdited => "request-identity-edited",
            Self::RequestInviteeLeft => "request-invitee-left",
            Self::RequestRevised => "request-revised",
            Self::RequestChecksUpdated => "request-checks-updated",
            Self::RequestStackUpdated => "request-stack-updated",
            Self::MemberAdded => "member-added",
            Self::InviteUpdated => "invite-updated",
            Self::MemberPermissionsChanged => "member-permissions-changed",
            Self::InviteRevoked => "invite-revoked",
            Self::MemberRemoved => "member-removed",
            Self::ContributorDeleted => "contributor-deleted",
            Self::FirstPushApplied => "first-push-applied",
            Self::PushReceived => "push-received",
            Self::RequestSubmitted => "request-submitted",
            Self::RequestRated => "request-rated",
            Self::RequestInviteeAdded => "request-invitee-added",
            Self::RequestInviteeRemoved => "request-invitee-removed",
            Self::RequestAttentionChanged => "request-attention-changed",
            Self::GitHubConnectionChanged => "github-connection-changed",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RepoChangeBus {
    channels: Arc<Mutex<BTreeMap<String, broadcast::Sender<RepoChangeEvent>>>>,
    origin_id: Arc<str>,
}

impl Default for RepoChangeBus {
    fn default() -> Self {
        Self {
            channels: Arc::new(Mutex::new(BTreeMap::new())),
            origin_id: Arc::from(new_origin_id()),
        }
    }
}

impl RepoChangeBus {
    #[expect(clippy::expect_used)]
    fn channels(
        &self,
    ) -> std::sync::MutexGuard<'_, BTreeMap<String, broadcast::Sender<RepoChangeEvent>>> {
        self.channels
            .lock()
            .expect("repo change bus holders do not panic, so the lock is never poisoned")
    }

    pub(crate) fn origin_id(&self) -> &str {
        &self.origin_id
    }

    pub(crate) fn subscribe(&self, repo_id: &str) -> broadcast::Receiver<RepoChangeEvent> {
        let mut channels = self.channels();
        let sender = channels
            .entry(repo_id.to_string())
            .or_insert_with(|| broadcast::channel(REPO_CHANGE_CHANNEL_CAPACITY).0);
        sender.subscribe()
    }

    pub(crate) fn remove_if_idle(&self, repo_id: &str) {
        let mut channels = self.channels();
        if channels
            .get(repo_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            channels.remove(repo_id);
        }
    }

    pub(crate) fn publish_event(&self, event: RepoChangeEvent) {
        let mut channels = self.channels();
        let Some(sender) = channels.get(&event.repo_id).cloned() else {
            return;
        };
        if sender.receiver_count() == 0 || sender.send(event.clone()).is_err() {
            channels.remove(&event.repo_id);
        }
    }

    pub(crate) fn notification_payload(
        &self,
        event: &RepoChangeEvent,
    ) -> Result<String, serde_json::Error> {
        serde_json::to_string(&RepoChangeNotification {
            event: event.clone(),
            origin_id: self.origin_id().to_string(),
        })
    }

    pub(crate) fn publish_notification_payload(&self, payload: &str) {
        match serde_json::from_str::<RepoChangeNotification>(payload) {
            Ok(notification) if notification.origin_id != self.origin_id() => {
                self.publish_event(notification.event);
            }
            Ok(_) => {}
            Err(error) => tracing::warn!(
                %error,
                payload,
                "ignored malformed repo change notification"
            ),
        }
    }
}

pub(crate) fn repository_change_event(
    incarnation: &RepositoryIncarnation,
    version: u64,
    reason: RepoChangeReason,
) -> RepoChangeEvent {
    RepoChangeEvent {
        repo_id: incarnation.repository_id().to_string(),
        incarnation_id: incarnation.incarnation_id().to_string(),
        version,
        kind: match reason {
            RepoChangeReason::Connected => RepoChangeKind::Connected,
            RepoChangeReason::Lagged => RepoChangeKind::Lagged,
            reason @ (RepoChangeReason::Redacted
            | RepoChangeReason::RepoDeleted
            | RepoChangeReason::ConfigApplied
            | RepoChangeReason::MetadataUpdated
            | RepoChangeReason::RequestMerged
            | RepoChangeReason::RequestDeleted
            | RepoChangeReason::RequestClosed
            | RepoChangeReason::RequestStarted
            | RepoChangeReason::RequestIdentityEdited
            | RepoChangeReason::RequestInviteeLeft
            | RepoChangeReason::RequestRevised
            | RepoChangeReason::RequestChecksUpdated
            | RepoChangeReason::RequestStackUpdated
            | RepoChangeReason::MemberAdded
            | RepoChangeReason::InviteUpdated
            | RepoChangeReason::MemberPermissionsChanged
            | RepoChangeReason::InviteRevoked
            | RepoChangeReason::MemberRemoved
            | RepoChangeReason::ContributorDeleted
            | RepoChangeReason::FirstPushApplied
            | RepoChangeReason::PushReceived
            | RepoChangeReason::RequestSubmitted
            | RepoChangeReason::RequestRated
            | RepoChangeReason::RequestInviteeAdded
            | RepoChangeReason::RequestInviteeRemoved
            | RepoChangeReason::RequestAttentionChanged
            | RepoChangeReason::GitHubConnectionChanged) => RepoChangeKind::RepositoryChanged {
                reason: reason.as_str().to_string(),
            },
        },
    }
}

pub(crate) fn request_timeline_change_event(
    incarnation: &RepositoryIncarnation,
    request_id: String,
    discussion_id: String,
    through_position: u64,
    view: ViewId,
) -> RepoChangeEvent {
    RepoChangeEvent {
        repo_id: incarnation.repository_id().to_string(),
        incarnation_id: incarnation.incarnation_id().to_string(),
        version: 0,
        kind: RepoChangeKind::RequestTimelineChanged {
            request_id,
            discussion_id,
            through_position,
            view: view.into(),
        },
    }
}

impl crate::state::AppState {
    pub(crate) async fn repository_incarnation(
        &self,
        repo_id: &str,
    ) -> Result<Option<RepositoryIncarnation>, crate::error::ApiError> {
        Ok(self
            .metadata
            .repositories()
            .repository_record(repo_id)
            .await?
            .map(|record| record.incarnation()))
    }

    pub(crate) fn publish_repo_change(
        &self,
        incarnation: &RepositoryIncarnation,
        version: u64,
        reason: RepoChangeReason,
    ) {
        let event = repository_change_event(incarnation, version, reason);
        self.publish_repo_event(event, "repo change");
    }

    pub(crate) fn publish_request_summary_refresh(
        &self,
        incarnation: &RepositoryIncarnation,
        reason: RepoChangeReason,
    ) {
        self.publish_repo_change(incarnation, REQUEST_SUMMARY_REFRESH_VERSION, reason);
    }

    pub(crate) async fn publish_request_state_refresh(
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
        self.publish_known_request_state_refresh(incarnation, &request);
    }

    pub(crate) fn publish_known_request_state_refresh(
        &self,
        incarnation: &RepositoryIncarnation,
        request: &scope_domain::requests::Request,
    ) {
        let event = RepoChangeEvent {
            repo_id: incarnation.repository_id().to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            version: REQUEST_SUMMARY_REFRESH_VERSION,
            kind: RepoChangeKind::RequestStateChanged {
                request_id: request.id.clone(),
                view: request.view.clone().into(),
            },
        };
        self.publish_repo_event(event, "request state");
    }

    pub(crate) fn publish_request_timeline_change(
        &self,
        incarnation: &RepositoryIncarnation,
        request_id: String,
        discussion_id: String,
        through_position: u64,
        view: scope_domain::views::ViewId,
    ) {
        let event = request_timeline_change_event(
            incarnation,
            request_id,
            discussion_id,
            through_position,
            view,
        );
        self.publish_repo_event(event, "request discussion");
    }

    pub(crate) fn publish_github_workflow_runs_change(&self, incarnation: &RepositoryIncarnation) {
        let event = RepoChangeEvent {
            repo_id: incarnation.repository_id().to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            version: 0,
            kind: RepoChangeKind::GitHubWorkflowRunsChanged,
        };
        self.publish_repo_event(event, "GitHub workflow run");
    }

    pub(crate) fn publish_github_workflow_run_change(
        &self,
        incarnation: &RepositoryIncarnation,
        github_run_id: u64,
    ) {
        let event = RepoChangeEvent {
            repo_id: incarnation.repository_id().to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            version: 0,
            kind: RepoChangeKind::GitHubWorkflowRunChanged { github_run_id },
        };
        self.publish_repo_event(event, "GitHub workflow job");
    }

    fn publish_repo_event(&self, event: RepoChangeEvent, description: &'static str) {
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

fn new_origin_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incarnation(repo_id: &str) -> RepositoryIncarnation {
        RepositoryIncarnation::new(repo_id, format!("repoi_{repo_id}"))
            .expect("test repository identity is valid")
    }

    fn event(repo_id: &str) -> RepoChangeEvent {
        repository_change_event(&incarnation(repo_id), 4, RepoChangeReason::PushReceived)
    }

    #[test]
    fn notification_payload_suppresses_same_origin_and_forwards_other_origins() {
        let bus = RepoChangeBus::default();
        let mut receiver = bus.subscribe("repo");

        let local_payload = bus.notification_payload(&event("repo")).unwrap();
        bus.publish_notification_payload(&local_payload);
        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));

        let external_payload = serde_json::to_string(&RepoChangeNotification {
            event: event("repo"),
            origin_id: "another-process".to_string(),
        })
        .unwrap();
        bus.publish_notification_payload(&external_payload);
        assert_eq!(receiver.try_recv().unwrap(), event("repo"));
    }

    #[test]
    fn idle_channels_are_removed_and_recreated_on_subscribe() {
        let bus = RepoChangeBus::default();
        let receiver = bus.subscribe("repo");
        drop(receiver);
        bus.remove_if_idle("repo");

        let mut replacement = bus.subscribe("repo");
        bus.publish_event(event("repo"));

        assert_eq!(replacement.try_recv().unwrap(), event("repo"));
    }
}
