use crate::{
    auth::scope::optional_scope_user,
    error::ApiError,
    repo_access::find_read_access,
    repo_events::{RepoChangeEvent, RepoChangeKind, RepoChangeReason, repository_change_event},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream;
use scope_domain::{
    account::UserAccount,
    repository::access::{RepositoryAccessContext, RepositoryActor},
    repository::{RepositoryIncarnation, repo_id},
    requests::{RequestViewer, request_policy},
};
use std::{convert::Infallible, time::Duration};
use tokio_stream::{Stream, StreamExt, once, wrappers::BroadcastStream};

const CLIENT_RESYNC_VERSION: u64 = 9_007_199_254_740_991;
const RECONCILIATION_INTERVAL: Duration = Duration::from_secs(30);

pub(crate) async fn repo_events(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let user = optional_scope_user(&state, &headers).await?;
    let repo_id = repo_id(&owner, &repo_name);
    let receiver = state.repo_events.subscribe(&repo_id);

    let repo = match find_read_access(
        &state,
        &owner,
        &repo_name,
        user.as_ref().map(|user| user.id.as_str()),
    )
    .await
    {
        Ok(repo) => repo,
        Err(error) => {
            drop(receiver);
            state.repo_events.remove_if_idle(&repo_id);
            return Err(error);
        }
    };
    let incarnation = repo.incarnation();
    let initial = event_for_access(
        &repo,
        repository_change_event(
            &incarnation,
            repo.record.change_version,
            RepoChangeReason::Connected,
        ),
    )
    .expect("connected event is always visible");
    let updates = stream::unfold(
        RepoEventStreamState {
            finished: false,
            headers,
            incarnation,
            owner,
            receiver: BroadcastStream::new(receiver),
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
                            return Some((sse_error_event(error), stream_state));
                        }
                        // NOTIFY is only a fast path. Reconcile from committed data even
                        // when a writer was cancelled after commit or LISTEN missed it.
                        stream_state.resync_event()
                    }
                    event = stream_state.receiver.next() => match event? {
                        Ok(event) => event,
                        Err(_) => stream_state.resync_event(),
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
                    Ok(Some(event)) => return Some((sse_event(event), stream_state)),
                    Ok(None) => continue,
                    Err(error) => {
                        stream_state.finished = true;
                        return Some((sse_error_event(error), stream_state));
                    }
                }
            }
        },
    );

    Ok(
        Sse::new(once(sse_event(initial)).chain(updates)).keep_alive(
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
    receiver: BroadcastStream<RepoChangeEvent>,
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

    fn resync_event(&self) -> RepoChangeEvent {
        repository_change_event(
            &self.incarnation,
            CLIENT_RESYNC_VERSION,
            RepoChangeReason::Lagged,
        )
    }
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
    if let RepoChangeKind::RequestTimelineChanged { request_id, .. } = &event.kind {
        let Some(request) = state.metadata.requests().request_by_id(request_id).await? else {
            return Ok(None);
        };
        let user_id = user.map(|user| user.id.as_str());
        let is_invitee = match user_id {
            Some(user_id) => {
                state
                    .metadata
                    .requests()
                    .request_is_invitee(request_id, user_id)
                    .await?
            }
            None => false,
        };
        if !request_policy(
            &request,
            RequestViewer::new(repo.access, user_id, is_invitee),
        )
        .activity_stream_visible
        {
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
    if repo.access.actor != RepositoryActor::Public {
        return Some(event);
    }

    if matches!(&event.kind, RepoChangeKind::RunChanged { .. }) {
        return None;
    }

    if let RepoChangeKind::RequestTimelineChanged { audience, .. }
    | RepoChangeKind::RequestAttachmentChanged { audience, .. } = &event.kind
    {
        if matches!(audience, scope_api_contract::RequestAudience::Public) {
            return Some(RepoChangeEvent {
                version: 0,
                ..event
            });
        }
        return None;
    }

    Some(RepoChangeEvent {
        kind: match event.kind {
            RepoChangeKind::Connected => RepoChangeKind::Connected,
            RepoChangeKind::Lagged => RepoChangeKind::Lagged,
            _ => RepoChangeKind::RepositoryChanged {
                reason: "repo-changed".to_string(),
            },
        },
        repo_id: event.repo_id,
        incarnation_id: event.incarnation_id,
        version: 0,
    })
}

fn sse_event(event: RepoChangeEvent) -> Result<Event, Infallible> {
    let data = serde_json::to_string(&event).expect("repo change events must serialize");
    Ok(Event::default().event("repo-change").data(data))
}

fn sse_error_event(error: ApiError) -> Result<Event, Infallible> {
    let (_, body) = error.into_public_parts();
    let data = serde_json::to_string(&body).expect("public API errors must serialize");
    Ok(Event::default().event("error").data(data))
}
