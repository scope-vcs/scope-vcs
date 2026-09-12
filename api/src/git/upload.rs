use crate::{
    auth::scope::principal_for_user_id,
    config::{AWAITING_FIRST_PUSH_GIT_ERROR, GIT_UPLOAD_PACK},
    error::ApiError,
    git::{
        GitRemoteMode,
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::{git_command_output, git_command_output_with_timeout, truncated_git_stderr},
        git_read_scope_user,
        projection_repo::projection_bare_repo_for_state,
        request_refs::attach_visible_request_refs,
    },
    repo_access::{ensure_repo_read, find_repo},
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
use scope_domain::policy::Principal;
use scope_domain::{
    projection::{ProjectionViewKey, project_graph},
    repository::access::RepositoryActor,
    repository::{RepoLifecycleState, RepositoryIncarnation},
    requests::{Request, RequestViewer, request_policy},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use std::{
    fs,
    io::Read,
    path::Path as FsPath,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};
mod read_view_identity;
#[cfg(test)]
mod read_view_tests;
use read_view_identity::GitReadViewIdentity;
static GIT_READ_VIEW_CACHE_ATTEMPT: AtomicU64 = AtomicU64::new(1);

/// Resolves the repository, the caller's access and (when authenticated) the viewer for a
/// Git read over the given remote mode, refusing unpublished repositories.
pub(crate) async fn authorized_git_read(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<
    (
        scope_domain::repository::Repository,
        scope_domain::repository::access::RepositoryAccess,
        Option<String>,
    ),
    ApiError,
> {
    let (repo, principal, viewer_user_id) =
        match git_read_principal_for_request(state, headers, owner, repo_name, mode).await {
            Ok(value) => value,
            Err(error)
                if mode == GitRemoteMode::Public && error.status() == StatusCode::NOT_FOUND =>
            {
                return Err(ApiError::unauthorized("Git credentials required"));
            }
            Err(error) => return Err(error),
        };
    if repo.record.lifecycle_state != RepoLifecycleState::Ready {
        return Err(unpublished_git_read_error(
            &repo, owner, repo_name, &principal,
        ));
    }
    ensure_repo_read(state, &repo, &principal)?;
    let access = repo.access_for_principal(&principal);
    Ok((repo, access, viewer_user_id))
}

pub(crate) async fn git_upload_pack_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<GitRepoHandle, ApiError> {
    let (repo, access, viewer_user_id) =
        authorized_git_read(state, headers, owner, repo_name, mode).await?;
    let private_view = ProjectionViewKey::from_access(access) == ProjectionViewKey::Private;
    let base_repo = if private_view {
        match repo.git_head.as_ref() {
            Some(head) => {
                state
                    .repository_engine
                    .materialize_repository(state, &repo.incarnation(), head, &repo.git_pack_spans)
                    .await?
            }
            None => {
                let projection = project_graph(
                    &repo.graph,
                    &repo.visibility_change_sets,
                    ProjectionViewKey::Private,
                );
                projection_bare_repo_for_state(
                    state,
                    &repo.incarnation(),
                    &projection,
                    repo.git_head.as_ref(),
                    &repo.git_pack_spans,
                )
                .await?
            }
        }
    } else {
        let projection = project_graph(
            &repo.graph,
            &repo.visibility_change_sets,
            ProjectionViewKey::Public,
        );
        projection_bare_repo_for_state(
            state,
            &repo.incarnation(),
            &projection,
            repo.git_head.as_ref(),
            &repo.git_pack_spans,
        )
        .await?
    };
    let mut requests = Vec::new();
    for request in state
        .metadata
        .requests()
        .requests_by_repo_id(&repo.record.id)
        .await?
    {
        let is_invitee = match viewer_user_id.as_deref() {
            Some(user_id) => {
                state
                    .metadata
                    .requests()
                    .request_is_invitee(&request.id, user_id)
                    .await?
            }
            None => false,
        };
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
        let projection = project_graph(
            &repo.graph,
            &repo.visibility_change_sets,
            ProjectionViewKey::Public,
        );
        Some(
            projection_bare_repo_for_state(
                state,
                &repo.incarnation(),
                &projection,
                repo.git_head.as_ref(),
                &repo.git_pack_spans,
            )
            .await?,
        )
    } else {
        None
    };
    git_read_view_repo(
        state,
        &repo.incarnation(),
        base_repo,
        public_base_repo,
        &requests,
    )
    .await
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
    let repo_path = cache_root.join(format!("read-view-{cache_key}.git"));
    let repo_path_for_ready = repo_path.clone();
    let is_ready = move || repo_path_for_ready.join("objects").is_dir();
    let state_for_build = state.clone();
    let base_repo_for_build = base_repo;
    let public_base_repo_for_build = public_base_repo;
    let requests_for_build = requests.to_vec();
    let cache_root_for_build = cache_root.clone();
    let cache_key_for_build = cache_key.clone();
    let repo_path_for_build = repo_path.clone();
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
                    "read-view-{cache_key_for_build}.{}.{}.tmp",
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
                    attach_visible_request_refs(
                        &state_for_build,
                        &requests_for_build,
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

pub(crate) async fn git_read_principal_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    mode: GitRemoteMode,
) -> Result<
    (
        scope_domain::repository::Repository,
        Principal,
        Option<String>,
    ),
    ApiError,
> {
    match mode {
        GitRemoteMode::Public => {
            let repo = find_repo(state, owner, repo_name).await?;
            Ok((repo, Principal::public(), None))
        }
        GitRemoteMode::Permissioned => {
            let user = git_read_scope_user(state, headers).await?;
            let repo = find_repo(state, owner, repo_name).await?;
            let principal = principal_for_user_id(&repo, &user.id);
            Ok((repo, principal, Some(user.id)))
        }
    }
}

fn unpublished_git_read_error(
    repo: &scope_domain::repository::Repository,
    owner: &str,
    repo_name: &str,
    principal: &Principal,
) -> ApiError {
    if repo.access_for_principal(principal).actor == RepositoryActor::Owner {
        ApiError::forbidden(AWAITING_FIRST_PUSH_GIT_ERROR)
    } else {
        ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
    }
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
    tokio::spawn(async move {
        let error_sender = sender.clone();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let _repo = repo;
            let deadline = Instant::now() + timeout;
            let mut command = Command::new("git");
            command
                .arg("upload-pack")
                .arg("--stateless-rpc")
                .arg(repo_path);
            run_with_stdout(
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
            )
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
    });

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
