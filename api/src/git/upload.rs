use crate::{
    config::{AWAITING_FIRST_PUSH_GIT_ERROR, GIT_UPLOAD_PACK},
    error::ApiError,
    git::{
        GitRemoteMode,
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::{
            git_command_output, git_command_output_with_timeout, git_subprocess_span,
            record_git_exit, truncated_git_stderr,
        },
        git_read_scope_user,
        projection_repo::projection_bare_repo_for_state,
        request_refs::attach_visible_request_refs,
        storage::repository_storage_key,
    },
    runtime_budgets::RuntimePermit,
    state::AppState,
};
use axum::{
    body::{Body, Bytes},
    http::{
        HeaderMap, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use scope_domain::{
    projection::ProjectionViewKey,
    repository::access::{RepositoryAccessContext, RepositoryActor},
    repository::{RepoLifecycleState, RepositoryIncarnation},
    requests::{Request, RequestViewer, request_policy},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_postgres::db::{GitReadSource, RepositoryProjectionSource};
use std::{
    fs,
    io::Read,
    path::Path as FsPath,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
use tracing::Instrument as _;
mod read_view_identity;
mod read_view_seed;
#[cfg(test)]
mod read_view_tests;
use read_view_identity::GitReadViewIdentity;
use read_view_seed::seed_request_refs_from_read_views;
static GIT_READ_VIEW_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

pub(crate) async fn authorized_git_read(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<(GitReadSource, Option<String>), ApiError> {
    let viewer_user_id = match mode {
        GitRemoteMode::Public => None,
        GitRemoteMode::Permissioned => Some(git_read_scope_user(state, headers).await?.id),
    };
    let Some(source) = state
        .metadata
        .repositories()
        .git_read_source(owner, repo_name, viewer_user_id.as_deref())
        .await?
    else {
        return Err(match mode {
            GitRemoteMode::Public => ApiError::unauthorized("Git credentials required"),
            GitRemoteMode::Permissioned => repo_not_found(owner, repo_name),
        });
    };
    let context = &source.context;
    if context.record.lifecycle_state != RepoLifecycleState::Ready {
        return Err(if context.access.actor == RepositoryActor::Owner {
            ApiError::forbidden(AWAITING_FIRST_PUSH_GIT_ERROR)
        } else {
            repo_not_found(owner, repo_name)
        });
    }
    if !context.can_read(source.public_files_visible) {
        return Err(repo_not_found(owner, repo_name));
    }
    Ok((source, viewer_user_id))
}

fn repo_not_found(owner: &str, repo_name: &str) -> ApiError {
    ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
}

pub(crate) async fn git_upload_pack_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<GitRepoHandle, ApiError> {
    let (source, viewer_user_id) =
        authorized_git_read(state, headers, owner, repo_name, mode).await?;
    let GitReadSource {
        context,
        git_head,
        git_pack_spans,
        ..
    } = source;
    let incarnation = context.incarnation();
    let access = context.access;
    let view_key = ProjectionViewKey::from_access(access);
    let private_view = view_key == ProjectionViewKey::Private;
    let mut projection_source = None;
    let base_repo = match git_head.as_ref() {
        Some(head) if private_view => {
            state
                .repository_engine
                .materialize_repository(state, &incarnation, head, &git_pack_spans)
                .await?
        }
        _ => {
            let source =
                projection_source.insert(repository_projection_source(state, &context).await?);
            projection_bare_repo_for_state(
                state,
                &incarnation,
                &source.project(view_key),
                git_head.as_ref(),
                &git_pack_spans,
            )
            .await?
        }
    };
    let mut requests = Vec::new();
    for (request, is_invitee) in state
        .metadata
        .requests()
        .requests_with_invitee_status(&context.record.id, viewer_user_id.as_deref())
        .await?
    {
        let decision = request_policy(
            &request,
            RequestViewer::new(access, viewer_user_id.as_deref(), is_invitee),
        );
        if decision.exact_visible {
            requests.push(request);
        }
    }
    requests.sort_by(|left, right| left.name.cmp(&right.name));
    let public_base_repo = if private_view
        && requests.iter().any(|request| {
            request.audience == scope_domain::requests::RequestAudience::Public
                && request.git_snapshot.is_none()
        }) {
        let source = match projection_source {
            Some(source) => source,
            None => repository_projection_source(state, &context).await?,
        };
        Some(
            projection_bare_repo_for_state(
                state,
                &incarnation,
                &source.project(ProjectionViewKey::Public),
                git_head.as_ref(),
                &git_pack_spans,
            )
            .await?,
        )
    } else {
        None
    };
    git_read_view_repo(state, &incarnation, base_repo, public_base_repo, &requests).await
}

async fn repository_projection_source(
    state: &AppState,
    context: &RepositoryAccessContext,
) -> Result<RepositoryProjectionSource, ApiError> {
    Ok(state
        .metadata
        .repositories()
        .repository_projection_source(&context.incarnation(), context.record.content_version)
        .await?)
}

async fn git_read_view_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    base_repo: GitRepoHandle,
    public_base_repo: Option<GitRepoHandle>,
    requests: &[Request],
) -> Result<GitRepoHandle, ApiError> {
    if requests.is_empty() {
        return Ok(base_repo);
    }
    let base_repo_path = base_repo.as_ref().to_path_buf();
    let public_base_repo_path = public_base_repo.as_deref().map(FsPath::to_path_buf);
    let (main_oid, public_main_oid) = tokio::task::spawn_blocking(move || {
        let head = |path: &FsPath| {
            git_command_output(
                Command::new("git")
                    .arg("--git-dir")
                    .arg(path)
                    .arg("rev-parse")
                    .arg(format!("refs/heads/{DEFAULT_GIT_BRANCH}")),
                None,
            )
        };
        Ok::<_, ApiError>((
            head(&base_repo_path)?,
            public_base_repo_path.as_deref().map(head).transpose()?,
        ))
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("Git read-view identity task failed: {error}"))
    })??;
    let cache_key = GitReadViewIdentity::from_authorized_output(
        incarnation,
        &main_oid,
        public_main_oid.as_deref(),
        requests,
    )
    .cache_key();
    let cache_root = state.repository_engine.cache_root().to_path_buf();
    let read_view_prefix = repository_storage_key(incarnation);
    let read_view_name = format!("read-view-{read_view_prefix}-{cache_key}");
    let repo_path = cache_root.join(format!("{read_view_name}.git"));
    let repo_path_for_ready = repo_path.clone();
    let is_ready = move || repo_path_for_ready.join("objects").is_dir();
    let state_for_build = state.clone();
    let base_repo_for_build = base_repo;
    let public_base_repo_for_build = public_base_repo;
    let requests_for_build = requests.to_vec();
    let cache_root_for_build = cache_root.clone();
    let repo_path_for_build = repo_path.clone();
    let repository_id = incarnation.repository_id().to_string();
    state.repository_engine.materialize_derived(
        incarnation,
        GitDerivedCacheNamespace::RequestReadView,
        cache_key.clone(),
        &repo_path,
        is_ready,
        move || async move {
            let permit = state_for_build.runtime_budgets.try_git_materialization()?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let attempt = GIT_READ_VIEW_CACHE_ATTEMPT.fetch_add(1, Ordering::Relaxed);
                let temp_path = cache_root_for_build.join(format!(
                    "{read_view_name}.{}.{}.tmp",
                    std::process::id(),
                    attempt
                ));
                if temp_path.exists() {
                    fs::remove_dir_all(&temp_path).map_err(ApiError::internal)?;
                }
                let result = (|| {
                    git_command_output(
                        Command::new("git")
                            .arg("clone")
                            .arg("--bare")
                            .arg("--no-hardlinks")
                            .arg(base_repo_for_build.as_ref())
                            .arg(&temp_path),
                        None,
                    )?;
                    let seeded = seed_request_refs_from_read_views(
                        &state_for_build.repository_engine,
                        &read_view_prefix,
                        &requests_for_build,
                        &temp_path,
                    )?;
                    let unattached: Vec<Request> = requests_for_build
                        .iter()
                        .filter(|request| !seeded.contains(&request.name))
                        .cloned()
                        .collect();
                    tracing::info!(
                        repository_id,
                        seeded_refs = seeded.len(),
                        snapshot_downloads = unattached
                            .iter()
                            .filter(|request| request.git_snapshot.is_some())
                            .count(),
                        "attaching request refs to Git read view"
                    );
                    attach_visible_request_refs(
                        &state_for_build,
                        &unattached,
                        &temp_path,
                        public_base_repo_for_build.as_deref(),
                    )?;
                    match fs::rename(&temp_path, &repo_path_for_build) {
                        Ok(()) => Ok(()),
                        Err(error) if repo_path_for_build.join("objects").is_dir() => {
                            tracing::debug!(%error, path = %repo_path_for_build.display(), "using externally-created Git read view cache");
                            Ok(())
                        }
                        Err(error) => Err(ApiError::internal(error)),
                    }
                })();
                let _ = fs::remove_dir_all(&temp_path);
                result
            })
            .await
            .map_err(|error| {
                ApiError::internal_message(format!(
                    "Git read-view materialization task failed: {error}"
                ))
            })?
        },
    )
    .await
}

pub(crate) async fn git_upload_pack_response(
    repo: GitRepoHandle,
    request: &[u8],
    timeout: Duration,
    permit: RuntimePermit,
) -> Result<Response, ApiError> {
    let repo_path = repo.as_ref().to_path_buf();
    let request = request.to_vec();
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let work = async move {
        let error_sender = sender.clone();
        let blocking_span = tracing::Span::current();
        let result = tokio::task::spawn_blocking(move || {
            let _entered = blocking_span.enter();
            let _permit = permit;
            let _repo = repo;
            let deadline = Instant::now() + timeout;
            let mut command = Command::new("git");
            command
                .arg("upload-pack")
                .arg("--stateless-rpc")
                .arg(repo_path);
            let git_span = git_subprocess_span(&command);
            let _entered = git_span.enter();
            let output = run_with_stdout(
                &mut command,
                Some(request),
                ProcessLimits::new(timeout),
                "Git upload-pack",
                move |mut stdout, _cancellation| {
                    let mut buffer = vec![0_u8; 64 * 1024];
                    loop {
                        let read = stdout.read(&mut buffer)?;
                        if read == 0 {
                            return Ok::<_, std::io::Error>(());
                        }
                        send_upload_pack_chunk(
                            &sender,
                            Bytes::copy_from_slice(&buffer[..read]),
                            deadline,
                        )?;
                    }
                },
            );
            if let Ok(output) = &output {
                record_git_exit(&git_span, output.status);
            }
            output
        })
        .await;
        let stream_error = match result {
            Ok(Ok(output)) if output.status.success() => None,
            Ok(Ok(output)) => Some(format!(
                "Git upload-pack failed: {}",
                truncated_git_stderr(&output.stderr)
            )),
            Ok(Err(StreamingProcessError::Consumer(error)))
                if error.kind() == std::io::ErrorKind::BrokenPipe =>
            {
                None
            }
            Ok(Err(error)) => Some(error.to_string()),
            Err(error) => Some(format!("Git upload-pack task failed: {error}")),
        };
        if let Some(message) = stream_error {
            let _ = error_sender.try_send(Err(std::io::Error::other(message)));
        }
    };
    tokio::spawn(work.in_current_span());

    Ok((
        StatusCode::OK,
        [
            (CONTENT_TYPE, "application/x-git-upload-pack-result"),
            (CACHE_CONTROL, "no-cache"),
        ],
        Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(receiver)),
    )
        .into_response())
}

fn send_upload_pack_chunk(
    sender: &tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>,
    chunk: Bytes,
    deadline: Instant,
) -> std::io::Result<()> {
    let mut item = Ok(chunk);
    loop {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "Git upload-pack client stopped reading before the command deadline",
            ));
        }
        match sender.try_send(item) {
            Ok(()) => return Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Git upload-pack client disconnected",
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(returned)) => item = returned,
        }
        std::thread::sleep(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(1)),
        );
    }
}

pub(crate) fn git_upload_pack_advertisement(repo_path: &FsPath, timeout: Duration) -> Response {
    match git_command_output_with_timeout(
        Command::new("git")
            .arg("upload-pack")
            .arg("--stateless-rpc")
            .arg("--advertise-refs")
            .arg(repo_path),
        None,
        timeout,
    ) {
        Ok(advertisement) => {
            let mut body = pkt_line(format!("# service={GIT_UPLOAD_PACK}\n").as_bytes());
            body.extend_from_slice(b"0000");
            body.extend(advertisement);
            git_response("application/x-git-upload-pack-advertisement", body)
        }
        Err(error) => git_advertisement_error(error.into_public_message()),
    }
}

pub(crate) fn git_response(content_type: &'static str, body: Vec<u8>) -> Response {
    (
        StatusCode::OK,
        [(CONTENT_TYPE, content_type), (CACHE_CONTROL, "no-cache")],
        Body::from(body),
    )
        .into_response()
}

pub(crate) fn git_advertisement_error(message: impl AsRef<str>) -> Response {
    git_response(
        "application/x-git-upload-pack-advertisement",
        git_error_body(message.as_ref()),
    )
}

pub(crate) fn git_upload_pack_error(message: impl AsRef<str>) -> Response {
    git_response(
        "application/x-git-upload-pack-result",
        git_error_body(message.as_ref()),
    )
}

pub(crate) fn git_error_body(message: &str) -> Vec<u8> {
    pkt_line(format!("ERR {message}\n").as_bytes())
}

pub(crate) fn pkt_line(payload: &[u8]) -> Vec<u8> {
    let len = payload.len() + 4;
    let mut line = format!("{len:04x}").into_bytes();
    line.extend_from_slice(payload);
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_pack_chunk_send_stops_at_deadline_when_live_receiver_is_full() {
        let (sender, _receiver) = tokio::sync::mpsc::channel(1);
        sender
            .try_send(Ok(Bytes::from_static(b"already full")))
            .unwrap();
        let started_at = Instant::now();

        let error = send_upload_pack_chunk(
            &sender,
            Bytes::from_static(b"blocked"),
            started_at + Duration::from_millis(25),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started_at.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn upload_pack_chunk_send_stops_when_receiver_is_closed() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);

        let error = send_upload_pack_chunk(
            &sender,
            Bytes::from_static(b"orphaned"),
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }
}
