use scope_api_contract::RepoChangeNotification;
pub(crate) use scope_api_contract::{RepoChangeEvent, RepoChangeKind, RunChangeKind};
use scope_domain::repository::RepositoryIncarnation;
use scope_domain::views::ViewId;
use scope_postgres::db::MetadataStore;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

const REPO_CHANGE_CHANNEL_CAPACITY: usize = 128;
const RUN_LOG_NOTIFY_WINDOW: Duration = Duration::from_millis(250);
pub(crate) const REQUEST_SUMMARY_REFRESH_VERSION: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogNotifyAction {
    Publish,
    Schedule(Duration),
    Pending,
}

#[derive(Clone, Copy, Debug)]
struct LogNotifyWindow {
    last_published: Option<Instant>,
    trailing_due: Option<Instant>,
}

#[derive(Debug)]
struct LogNotifyRegistry {
    windows: HashMap<String, LogNotifyWindow>,
    prune_after: Instant,
}

impl Default for LogNotifyRegistry {
    fn default() -> Self {
        Self {
            windows: HashMap::new(),
            prune_after: Instant::now() + Duration::from_secs(30),
        }
    }
}

impl LogNotifyWindow {
    fn on_append(&mut self, now: Instant) -> LogNotifyAction {
        match self.last_published {
            Some(last) if now < last + RUN_LOG_NOTIFY_WINDOW => {
                if self.trailing_due.is_some() {
                    LogNotifyAction::Pending
                } else {
                    let due = last + RUN_LOG_NOTIFY_WINDOW;
                    self.trailing_due = Some(due);
                    LogNotifyAction::Schedule(due - now)
                }
            }
            _ => {
                self.last_published = Some(now);
                self.trailing_due = None;
                LogNotifyAction::Publish
            }
        }
    }

    fn on_deadline(&mut self, now: Instant) -> bool {
        if self.trailing_due.is_some_and(|due| now >= due) {
            self.last_published = Some(now);
            self.trailing_due = None;
            true
        } else {
            false
        }
    }
}

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
    NativeRunsChanged,
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
            Self::NativeRunsChanged => "native-runs-changed",
            Self::GitHubConnectionChanged => "github-connection-changed",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RepoChangeBus {
    channels: Arc<Mutex<BTreeMap<String, broadcast::Sender<RepoChangeEvent>>>>,
    log_notifications: Arc<Mutex<LogNotifyRegistry>>,
    origin_id: Arc<str>,
}

impl Default for RepoChangeBus {
    fn default() -> Self {
        Self {
            channels: Arc::new(Mutex::new(BTreeMap::new())),
            log_notifications: Arc::new(Mutex::new(LogNotifyRegistry::default())),
            origin_id: Arc::from(new_origin_id()),
        }
    }
}

impl RepoChangeBus {
    fn on_log_append(&self, run_id: &str, now: Instant) -> LogNotifyAction {
        let mut registry = self
            .log_notifications
            .lock()
            .expect("run log notification lock must not be poisoned");
        if now >= registry.prune_after {
            registry.windows.retain(|_, window| {
                window.trailing_due.is_some()
                    || window
                        .last_published
                        .is_some_and(|last| now < last + RUN_LOG_NOTIFY_WINDOW)
            });
            registry.prune_after = now + Duration::from_secs(30);
        }
        registry
            .windows
            .entry(run_id.to_owned())
            .or_insert(LogNotifyWindow {
                last_published: None,
                trailing_due: None,
            })
            .on_append(now)
    }

    fn on_log_deadline(&self, run_id: &str, now: Instant) -> bool {
        self.log_notifications
            .lock()
            .expect("run log notification lock must not be poisoned")
            .windows
            .get_mut(run_id)
            .is_some_and(|window| window.on_deadline(now))
    }

    pub(crate) fn origin_id(&self) -> &str {
        &self.origin_id
    }

    pub(crate) fn subscribe(&self, repo_id: &str) -> broadcast::Receiver<RepoChangeEvent> {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        let sender = channels
            .entry(repo_id.to_string())
            .or_insert_with(|| broadcast::channel(REPO_CHANGE_CHANNEL_CAPACITY).0);
        sender.subscribe()
    }

    pub(crate) fn remove_if_idle(&self, repo_id: &str) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
        if channels
            .get(repo_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            channels.remove(repo_id);
        }
    }

    pub(crate) fn publish_event(&self, event: RepoChangeEvent) {
        let mut channels = self
            .channels
            .lock()
            .expect("repo change bus lock must not be poisoned");
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
            reason => RepoChangeKind::RepositoryChanged {
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

pub(crate) fn run_change_event(
    incarnation: &RepositoryIncarnation,
    run_id: String,
    change: RunChangeKind,
) -> RepoChangeEvent {
    RepoChangeEvent::run_changed(
        incarnation.repository_id().to_string(),
        incarnation.incarnation_id().to_string(),
        run_id,
        change,
    )
}

impl crate::state::AppState {
    pub(crate) async fn publish_repo_change(
        &self,
        incarnation: &RepositoryIncarnation,
        version: u64,
        reason: RepoChangeReason,
    ) {
        let event = repository_change_event(incarnation, version, reason);
        self.publish_repo_event(event, "repo change").await;
    }

    pub(crate) async fn publish_request_summary_refresh(
        &self,
        incarnation: &RepositoryIncarnation,
        reason: RepoChangeReason,
    ) {
        self.publish_repo_change(incarnation, REQUEST_SUMMARY_REFRESH_VERSION, reason)
            .await;
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
        self.publish_known_request_state_refresh(incarnation, &request)
            .await;
    }

    pub(crate) async fn publish_known_request_state_refresh(
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
        self.publish_repo_event(event, "request state").await;
    }

    pub(crate) async fn publish_request_timeline_change(
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
        self.publish_repo_event(event, "request discussion").await;
    }

    pub(crate) async fn publish_github_workflow_runs_change(
        &self,
        incarnation: &RepositoryIncarnation,
    ) {
        let event = RepoChangeEvent {
            repo_id: incarnation.repository_id().to_string(),
            incarnation_id: incarnation.incarnation_id().to_string(),
            version: 0,
            kind: RepoChangeKind::GitHubWorkflowRunsChanged,
        };
        self.publish_repo_event(event, "GitHub workflow run").await;
    }

    pub(crate) async fn publish_github_workflow_run_change(
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
        self.publish_repo_event(event, "GitHub workflow job").await;
    }

    pub(crate) async fn publish_run_change(
        &self,
        repo_id: &str,
        run_id: String,
        change: RunChangeKind,
    ) {
        if change == RunChangeKind::LogsAppended {
            match self.repo_events.on_log_append(&run_id, Instant::now()) {
                LogNotifyAction::Publish => {}
                LogNotifyAction::Pending => return,
                LogNotifyAction::Schedule(delay) => {
                    let state = self.clone();
                    let repo_id = repo_id.to_owned();
                    tokio::spawn(async move {
                        tokio::time::sleep(delay).await;
                        if state.repo_events.on_log_deadline(&run_id, Instant::now()) {
                            state
                                .publish_run_change_now(
                                    &repo_id,
                                    run_id,
                                    RunChangeKind::LogsAppended,
                                )
                                .await;
                        }
                    });
                    return;
                }
            }
        }
        self.publish_run_change_now(repo_id, run_id, change).await;
    }

    async fn publish_run_change_now(&self, repo_id: &str, run_id: String, change: RunChangeKind) {
        let incarnation = match self
            .metadata
            .repositories()
            .run_repository_incarnation(&run_id, repo_id)
            .await
        {
            Ok(Some(incarnation)) => incarnation,
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(repo_id, error = %error.message, "failed to resolve run notification repository incarnation");
                return;
            }
        };
        let event = run_change_event(&incarnation, run_id, change);
        self.publish_repo_event(event, "run change").await;
    }

    pub(crate) async fn publish_repo_event(
        &self,
        event: RepoChangeEvent,
        description: &'static str,
    ) {
        publish_repo_event(&self.repo_events, &self.metadata, event, description).await;
    }
}

pub(crate) async fn publish_repo_event(
    bus: &RepoChangeBus,
    metadata: &MetadataStore,
    event: RepoChangeEvent,
    description: &'static str,
) {
    bus.publish_event(event.clone());
    let payload = match bus.notification_payload(&event) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(repo_id = %event.repo_id, %error, "failed to serialize {description} notification");
            return;
        }
    };
    if let Err(error) = metadata.repositories().notify_repo_change(&payload).await {
        tracing::warn!(
            repo_id = %event.repo_id,
            error = %error.message,
            "failed to publish {description} notification"
        );
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

    #[test]
    fn log_notifications_publish_at_most_four_times_per_second_and_keep_the_trailing_edge() {
        let start = Instant::now();
        let mut window = LogNotifyWindow {
            last_published: None,
            trailing_due: None,
        };
        let mut published = Vec::new();
        for tick in 0..100 {
            let now = start + Duration::from_millis(tick * 10);
            if window.on_deadline(now) {
                published.push(now);
            }
            if window.on_append(now) == LogNotifyAction::Publish {
                published.push(now);
            }
        }
        assert_eq!(published.len(), 4);
        assert!(
            published
                .windows(2)
                .all(|times| times[1] - times[0] >= RUN_LOG_NOTIFY_WINDOW)
        );
        assert!(window.on_deadline(start + Duration::from_secs(1)));
        assert_eq!(window.last_published, Some(start + Duration::from_secs(1)));
        assert!(!window.on_deadline(start + Duration::from_millis(1_250)));
    }

    #[test]
    fn log_notification_windows_are_per_run_and_a_stale_timer_cannot_duplicate_a_publish() {
        let bus = RepoChangeBus::default();
        let start = Instant::now();
        assert_eq!(bus.on_log_append("first", start), LogNotifyAction::Publish);
        assert_eq!(bus.on_log_append("second", start), LogNotifyAction::Publish);
        assert_eq!(
            bus.on_log_append("first", start + Duration::from_millis(100)),
            LogNotifyAction::Schedule(Duration::from_millis(150))
        );
        assert_eq!(
            bus.on_log_append("first", start + Duration::from_millis(200)),
            LogNotifyAction::Pending
        );
        assert_eq!(
            bus.on_log_append("first", start + Duration::from_millis(250)),
            LogNotifyAction::Publish
        );
        assert!(!bus.on_log_deadline("first", start + Duration::from_millis(250)));
    }

    fn incarnation(repo_id: &str) -> RepositoryIncarnation {
        RepositoryIncarnation::new(repo_id, format!("repoi_{repo_id}"))
            .expect("test repository identity is valid")
    }

    fn event(repo_id: &str) -> RepoChangeEvent {
        repository_change_event(&incarnation(repo_id), 4, RepoChangeReason::PushReceived)
    }

    #[test]
    fn typed_reasons_preserve_special_kinds_and_wire_reason() {
        assert_eq!(
            repository_change_event(&incarnation("repo"), 1, RepoChangeReason::Connected).kind,
            RepoChangeKind::Connected
        );
        assert_eq!(
            repository_change_event(&incarnation("repo"), 1, RepoChangeReason::Lagged).kind,
            RepoChangeKind::Lagged
        );
        assert_eq!(
            repository_change_event(&incarnation("repo"), 1, RepoChangeReason::ConfigApplied).kind,
            RepoChangeKind::RepositoryChanged {
                reason: "config-applied".to_string(),
            }
        );
    }

    #[test]
    fn run_changes_preserve_the_run_and_change_kind() {
        assert_eq!(
            run_change_event(
                &incarnation("repo"),
                "run_1".to_string(),
                RunChangeKind::LogsAppended
            ),
            RepoChangeEvent {
                repo_id: "repo".to_string(),
                incarnation_id: "repoi_repo".to_string(),
                version: 0,
                kind: RepoChangeKind::RunChanged {
                    run_id: "run_1".to_string(),
                    change: RunChangeKind::LogsAppended,
                },
            }
        );
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
    fn malformed_notifications_are_dropped() {
        let bus = RepoChangeBus::default();
        let mut receiver = bus.subscribe("repo");

        bus.publish_notification_payload("not-json");

        assert!(matches!(
            receiver.try_recv(),
            Err(broadcast::error::TryRecvError::Empty)
        ));
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
