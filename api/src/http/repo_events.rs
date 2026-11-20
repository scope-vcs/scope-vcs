use crate::{
    auth::scope::optional_scope_user,
    error::ApiError,
    repo_access::find_read_access,
    repo_events::{
        RepoChangeEvent, RepoChangeKind, RepoChangeReason, repository_change_event, synced_event,
        unix_now_ms,
    },
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream;
use scope_domain::{
    account::UserAccount,
    repository::access::RepositoryAccessContext,
    repository::{RepositoryIncarnation, repo_id},
    requests::request_policy,
};
use serde::Deserialize;
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast::{self, error::RecvError};
use tokio_stream::{Stream, StreamExt, iter};

const CLIENT_RESYNC_VERSION: u64 = 9_007_199_254_740_991;
const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Deserialize)]
pub(crate) struct RepoEventsQuery {
    since: Option<u64>,
}

pub(crate) async fn repo_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RepoEventsQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let repo_id = repo_id(&owner, &repo_name);
    let receiver = state.repo_events.subscribe(&repo_id);

    let catch_up = async {
        let repo = find_read_access(
            &state,
            &owner,
            &repo_name,
            user.as_ref().map(|user| user.id.as_str()),
        )
        .await?;
        let incarnation = repo.incarnation();
        let caught_up_ms = unix_now_ms();
        let mut initial = Vec::new();
        if let Some(since) = query.since {
            match state.repo_events.changes_since(&repo_id, since) {
                Some(missed) => {
                    for event in missed {
                        initial.extend(
                            stream_event_for_user(
                                &state,
                                &owner,
                                &repo_name,
                                &incarnation,
                                user.as_ref(),
                                event,
                            )
                            .await?,
                        );
                    }
                }
                None => initial.extend(event_for_access(&repo, lagged_event(&incarnation))),
            }
        }
        initial.extend(event_for_access(
            &repo,
            synced_event(&incarnation, caught_up_ms),
        ));
        Ok::<_, ApiError>((incarnation, initial))
    };
    let (incarnation, initial) = match catch_up.await {
        Ok(caught_up) => caught_up,
        Err(error) => {
            drop(receiver);
            state.repo_events.remove_if_idle(&repo_id);
            return Err(error);
        }
    };
    let updates = stream::unfold(
        RepoEventStreamState {
            finished: false,
            headers,
            incarnation,
            owner,
            receiver,
            reconciliation: reconciliation_interval(),
            repo_name,
            state: state.clone(),
            user,
        },
        |mut stream_state| async move {
            if stream_state.finished {
                return None;
            }
            loop {
                let event = tokio::select! {
                    biased;
                    _ = stream_state.reconciliation.tick() => {
                        if let Err(error) = stream_state.revalidate_authentication().await {
                            stream_state.finished = true;
                            return Some((Ok(sse_error_event(error)), stream_state));
                        }
                        if !stream_state.receiver.is_empty() {
                            continue;
                        }
                        synced_event(&stream_state.incarnation, unix_now_ms())
                    }
                    event = stream_state.receiver.recv() => match event {
                        Ok(event) => event,
                        Err(RecvError::Lagged(_)) => lagged_event(&stream_state.incarnation),
                        Err(RecvError::Closed) => return None,
                    },
                };

                match stream_event_for_user(
                    &stream_state.state,
                    &stream_state.owner,
                    &stream_state.repo_name,
                    &stream_state.incarnation,
                    stream_state.user.as_ref(),
                    event,
                )
                .await
                {
                    Ok(Some(event)) => return Some((Ok(sse_event(event)), stream_state)),
                    Ok(None) => continue,
                    Err(error) => {
                        stream_state.finished = true;
                        return Some((Ok(sse_error_event(error)), stream_state));
                    }
                }
            }
        },
    );

    Ok(
        Sse::new(iter(initial.into_iter().map(|event| Ok(sse_event(event)))).chain(updates))
            .keep_alive(
                KeepAlive::new()
                    .interval(Duration::from_secs(20))
                    .text("keep-alive"),
            ),
    )
}

struct RepoEventStreamState {
    finished: bool,
    headers: HeaderMap,
    incarnation: RepositoryIncarnation,
    owner: String,
    receiver: broadcast::Receiver<RepoChangeEvent>,
    reconciliation: tokio::time::Interval,
    repo_name: String,
    state: AppState,
    user: Option<UserAccount>,
}

impl RepoEventStreamState {
    async fn revalidate_authentication(&mut self) -> Result<(), ApiError> {
        let current_user = optional_scope_user(&self.state, &self.headers).await?;
        if current_user.as_ref().map(|user| &user.id) != self.user.as_ref().map(|user| &user.id) {
            return Err(ApiError::unauthorized(
                "event stream identity changed; reconnect",
            ));
        }
        self.user = current_user;
        Ok(())
    }
}

fn lagged_event(incarnation: &RepositoryIncarnation) -> RepoChangeEvent {
    repository_change_event(incarnation, CLIENT_RESYNC_VERSION, RepoChangeReason::Lagged)
}

fn reconciliation_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval_at(
        tokio::time::Instant::now() + RECONCILIATION_INTERVAL,
        RECONCILIATION_INTERVAL,
    );
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    interval
}

async fn stream_event_for_user(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    expected_incarnation: &RepositoryIncarnation,
    user: Option<&UserAccount>,
    event: RepoChangeEvent,
) -> Result<Option<RepoChangeEvent>, ApiError> {
    let repo = find_read_access(state, owner, repo_name, user.map(|user| user.id.as_str())).await?;
    if repo.incarnation() != *expected_incarnation {
        return Err(ApiError::conflict(
            "repository was recreated; reconnect event stream",
        ));
    }
    if event.repo_id != expected_incarnation.repository_id()
        || event.incarnation_id != expected_incarnation.incarnation_id()
    {
        return Ok(None);
    }
    if let RepoChangeKind::RequestTimelineChanged { request_id, .. }
    | RepoChangeKind::RequestStateChanged { request_id, .. } = &event.kind
    {
        let Some(request) = state.metadata.requests().request_by_id(request_id).await? else {
            return Ok(None);
        };
        let viewer = super::requests::request_viewer(
            state,
            request_id,
            repo.access.clone(),
            user.map(|user| user.id.as_str()),
        )
        .await?;
        let policy = request_policy(&request, viewer, &repo.views);
        let visible = match &event.kind {
            RepoChangeKind::RequestStateChanged { .. } => policy.exact_visible,
            RepoChangeKind::Synced { .. }
            | RepoChangeKind::Lagged
            | RepoChangeKind::DependenciesChanged
            | RepoChangeKind::GitHubWorkflowRunsChanged
            | RepoChangeKind::GitHubWorkflowRunChanged { .. }
            | RepoChangeKind::GitHubWorkflowRunsExpired
            | RepoChangeKind::RepositoryChanged { .. }
            | RepoChangeKind::RequestTimelineChanged { .. }
            | RepoChangeKind::RequestAttachmentChanged { .. } => policy.activity_stream_visible,
        };
        if !visible {
            return Ok(None);
        }
    }
    if let RepoChangeKind::RequestAttachmentChanged {
        request_id,
        attachment_id,
        ..
    } = &event.kind
        && state
            .metadata
            .media()
            .request_attachment_for_viewer(
                request_id,
                attachment_id,
                user.map(|user| user.id.as_str()),
            )
            .await?
            .is_none()
    {
        return Ok(None);
    }
    Ok(event_for_access(&repo, event))
}

fn event_for_access(
    repo: &RepositoryAccessContext,
    event: RepoChangeEvent,
) -> Option<RepoChangeEvent> {
    if repo.reads_full_view() {
        return Some(event);
    }

    if matches!(
        &event.kind,
        RepoChangeKind::DependenciesChanged
            | RepoChangeKind::GitHubWorkflowRunsChanged
            | RepoChangeKind::GitHubWorkflowRunChanged { .. }
            | RepoChangeKind::GitHubWorkflowRunsExpired
    ) {
        return None;
    }

    if let RepoChangeKind::RequestStateChanged { view, .. }
    | RepoChangeKind::RequestTimelineChanged { view, .. }
    | RepoChangeKind::RequestAttachmentChanged { view, .. } = &event.kind
    {
        if repo.can_read_view(&view.clone().into()) {
            return Some(RepoChangeEvent {
                version: 0,
                ..event
            });
        }
        return None;
    }

    Some(redacted_event(event))
}

fn redacted_event(event: RepoChangeEvent) -> RepoChangeEvent {
    RepoChangeEvent {
        kind: match event.kind {
            kind @ (RepoChangeKind::Synced { .. } | RepoChangeKind::Lagged) => kind,
            RepoChangeKind::DependenciesChanged
            | RepoChangeKind::GitHubWorkflowRunsChanged
            | RepoChangeKind::GitHubWorkflowRunChanged { .. }
            | RepoChangeKind::GitHubWorkflowRunsExpired
            | RepoChangeKind::RepositoryChanged { .. }
            | RepoChangeKind::RequestStateChanged { .. }
            | RepoChangeKind::RequestTimelineChanged { .. }
            | RepoChangeKind::RequestAttachmentChanged { .. } => {
                RepoChangeKind::RepositoryChanged {
                    reason: RepoChangeReason::Redacted.as_str().to_string(),
                }
            }
        },
        repo_id: event.repo_id,
        incarnation_id: event.incarnation_id,
        version: 0,
    }
}

fn sse_event(event: RepoChangeEvent) -> Event {
    #[expect(clippy::expect_used)]
    let data = serde_json::to_string(&event)
        .expect("repo change events are plain structs that serialize as JSON");
    Event::default().event("repo-change").data(data)
}

fn sse_error_event(error: ApiError) -> Event {
    let (_, body) = error.into_public_parts();
    #[expect(clippy::expect_used)]
    let data = serde_json::to_string(&body)
        .expect("public API error bodies are plain structs that serialize as JSON");
    Event::default().event("error").data(data)
}
