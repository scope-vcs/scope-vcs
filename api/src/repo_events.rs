use scope_api_contract::RepoChangeNotification;
pub(crate) use scope_api_contract::{RepoChangeEvent, RepoChangeKind};
use scope_domain::repository::RepositoryIncarnation;
use scope_domain::views::ViewId;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::broadcast;

const REPO_CHANGE_CHANNEL_CAPACITY: usize = 128;
const RECENT_CHANGE_RETENTION_MS: u64 = 120_000;
const RECENT_CHANGES_PER_REPO: usize = 256;
const REPLAY_CLOCK_SKEW_MS: u64 = 5_000;
pub(crate) const REQUEST_SUMMARY_REFRESH_VERSION: u64 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoChangeReason {
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
    channels: Arc<Mutex<Option<RepoChannels>>>,
    recent: Arc<Mutex<RecentRepoChanges>>,
    origin_id: Arc<str>,
}

type RepoChannels = BTreeMap<String, broadcast::Sender<RepoChangeEvent>>;

impl Default for RepoChangeBus {
    fn default() -> Self {
        Self::complete_after(unix_now_ms())
    }
}

#[derive(Debug)]
struct RecentRepoChanges {
    complete_after_ms: u64,
    repos: BTreeMap<String, RecentChanges>,
}

#[derive(Debug, Default)]
struct RecentChanges {
    complete_after_ms: u64,
    events: VecDeque<(u64, RepoChangeEvent)>,
}

impl RecentRepoChanges {
    fn new(now_ms: u64) -> Self {
        Self {
            complete_after_ms: now_ms,
            repos: BTreeMap::new(),
        }
    }

    fn record(&mut self, event: &RepoChangeEvent, now_ms: u64) {
        let repo = self.repos.entry(event.repo_id.clone()).or_default();
        repo.events.push_back((now_ms, event.clone()));
        if repo.events.len() > RECENT_CHANGES_PER_REPO
            && let Some((dropped_at_ms, _)) = repo.events.pop_front()
        {
            repo.complete_after_ms = repo.complete_after_ms.max(dropped_at_ms);
        }
        let horizon_ms = now_ms.saturating_sub(RECENT_CHANGE_RETENTION_MS);
        self.repos.retain(|_, repo| {
            while repo
                .events
                .front()
                .is_some_and(|(received_at_ms, _)| *received_at_ms < horizon_ms)
            {
                repo.events.pop_front();
            }
            !repo.events.is_empty()
        });
    }

    fn since(&self, repo_id: &str, since_ms: u64, now_ms: u64) -> Option<Vec<RepoChangeEvent>> {
        let from_ms = since_ms.saturating_sub(REPLAY_CLOCK_SKEW_MS);
        let repo = self.repos.get(repo_id);
        let complete_after_ms = self
            .complete_after_ms
            .max(now_ms.saturating_sub(RECENT_CHANGE_RETENTION_MS))
            .max(repo.map_or(0, |repo| repo.complete_after_ms));
        if from_ms < complete_after_ms {
            return None;
        }
        Some(repo.map_or_else(Vec::new, |repo| {
            repo.events
                .iter()
                .filter(|(received_at_ms, _)| *received_at_ms > from_ms)
                .map(|(_, event)| event.clone())
                .collect()
        }))
    }

    fn forget(&mut self, now_ms: u64) {
        self.complete_after_ms = now_ms;
        self.repos.clear();
    }
}

impl RepoChangeBus {
    pub(crate) fn complete_after(unix_ms: u64) -> Self {
        Self {
            channels: Arc::new(Mutex::new(Some(BTreeMap::new()))),
            recent: Arc::new(Mutex::new(RecentRepoChanges::new(unix_ms))),
            origin_id: Arc::from(new_origin_id()),
        }
    }

    #[expect(clippy::expect_used)]
    fn channels(&self) -> std::sync::MutexGuard<'_, Option<RepoChannels>> {
        self.channels
            .lock()
            .expect("repo change bus holders do not panic, so the lock is never poisoned")
    }

    #[expect(clippy::expect_used)]
    fn recent(&self) -> std::sync::MutexGuard<'_, RecentRepoChanges> {
        self.recent
            .lock()
            .expect("recent change holders do not panic, so the lock is never poisoned")
    }

    pub(crate) fn origin_id(&self) -> &str {
        &self.origin_id
    }

    pub(crate) fn changes_since(
        &self,
        repo_id: &str,
        since_ms: u64,
    ) -> Option<Vec<RepoChangeEvent>> {
        self.recent().since(repo_id, since_ms, unix_now_ms())
    }

    pub(crate) fn notifications_lost(&self) {
        self.recent().forget(unix_now_ms());
        if let Some(channels) = self.channels().as_mut() {
            channels.clear();
        }
    }

    pub(crate) fn subscribe(&self, repo_id: &str) -> broadcast::Receiver<RepoChangeEvent> {
        match self.channels().as_mut() {
            Some(channels) => channels
                .entry(repo_id.to_string())
                .or_insert_with(|| broadcast::channel(REPO_CHANGE_CHANNEL_CAPACITY).0)
                .subscribe(),
            None => broadcast::channel(1).1,
        }
    }

    pub(crate) fn close(&self) {
        self.channels().take();
    }

    pub(crate) fn remove_if_idle(&self, repo_id: &str) {
        let mut channels = self.channels();
        let Some(channels) = channels.as_mut() else {
            return;
        };
        if channels
            .get(repo_id)
            .is_some_and(|sender| sender.receiver_count() == 0)
        {
            channels.remove(repo_id);
        }
    }

    pub(crate) fn publish_event(&self, event: RepoChangeEvent) {
        self.recent().record(&event, unix_now_ms());
        let mut channels = self.channels();
        let Some(channels) = channels.as_mut() else {
            return;
        };
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

pub(crate) fn synced_event(
    incarnation: &RepositoryIncarnation,
    through_unix_ms: u64,
) -> RepoChangeEvent {
    RepoChangeEvent {
        repo_id: incarnation.repository_id().to_string(),
        incarnation_id: incarnation.incarnation_id().to_string(),
        version: 0,
        kind: RepoChangeKind::Synced { through_unix_ms },
    }
}

pub(crate) fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
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
    fn closing_ends_open_subscriptions_after_delivered_events_and_later_ones_at_once() {
        let bus = RepoChangeBus::default();
        let mut open = bus.subscribe("repo");
        bus.publish_event(event("repo"));

        bus.close();
        bus.publish_event(event("repo"));

        assert_eq!(open.try_recv().unwrap(), event("repo"));
        for mut ended in [open, bus.subscribe("repo")] {
            assert!(matches!(
                ended.try_recv(),
                Err(broadcast::error::TryRecvError::Closed)
            ));
        }
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

    fn timeline(repo_id: &str, through_position: u64) -> RepoChangeEvent {
        request_timeline_change_event(
            &incarnation(repo_id),
            "request".to_string(),
            "discussion".to_string(),
            through_position,
            ViewId::private(),
        )
    }

    #[test]
    fn replay_returns_the_repository_changes_received_after_since_less_clock_skew() {
        let mut recent = RecentRepoChanges::new(0);
        recent.record(&timeline("repo", 1), 10_000);
        recent.record(&timeline("other", 2), 20_000);
        recent.record(&timeline("repo", 3), 30_000);

        assert_eq!(
            recent.since("repo", 25_000, 40_000),
            Some(vec![timeline("repo", 3)])
        );
        assert_eq!(
            recent.since("repo", 14_000, 40_000),
            Some(vec![timeline("repo", 1), timeline("repo", 3)])
        );
        assert_eq!(recent.since("quiet", 25_000, 40_000), Some(Vec::new()));
    }

    #[test]
    fn replay_is_refused_when_changes_since_then_may_be_missing() {
        let retained = |recent: &mut RecentRepoChanges| {
            recent.record(&timeline("repo", 1), 200_000);
        };
        let mut started_late = RecentRepoChanges::new(150_000);
        retained(&mut started_late);
        let mut overflowed = RecentRepoChanges::new(0);
        for position in 0..=u64::try_from(RECENT_CHANGES_PER_REPO).unwrap() {
            overflowed.record(&timeline("repo", position), 180_000 + position);
        }
        let mut forgotten = RecentRepoChanges::new(0);
        retained(&mut forgotten);
        forgotten.forget(190_000);

        for (case, recent, since_ms) in [
            ("since precedes this process", &started_late, 152_000),
            (
                "since precedes the retention window",
                &RecentRepoChanges::new(0),
                124_000,
            ),
            ("since precedes an overflowed change", &overflowed, 182_000),
            ("since precedes lost notifications", &forgotten, 192_000),
        ] {
            assert_eq!(recent.since("repo", since_ms, 250_000), None, "{case}");
        }
        assert!(started_late.since("repo", 160_000, 250_000).is_some());
    }
}
