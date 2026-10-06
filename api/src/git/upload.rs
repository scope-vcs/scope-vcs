use crate::{
    config::{AWAITING_FIRST_PUSH_GIT_ERROR, GIT_UPLOAD_PACK},
    error::ApiError,
    git::{
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::{
            git_command_output, git_command_output_with_timeout, git_subprocess_span,
            record_git_exit, truncated_git_stderr,
        },
        git_read_scope_user,
        repository_git::RepositoryGit,
        request_refs::attach_visible_request_refs,
        storage::repository_storage_key,
    },
    runtime_budgets::{RuntimeBudgets, RuntimePermit},
    state::AppState,
};
use axum::{
    body::{Body, Bytes},
    http::{
        HeaderMap, StatusCode,
        header::{AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE},
    },
    response::{IntoResponse, Response},
};
use scope_domain::{
    repository::access::RepositoryActor,
    repository::{RepoLifecycleState, RepositoryIncarnation},
    requests::{Request, RequestViewer, request_policy},
    views::ViewId,
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_postgres::db::GitReadSource;
use std::{
    collections::BTreeSet,
    fs,
    io::{BufRead, BufReader, Read},
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
    view: &ViewId,
) -> Result<(GitReadSource, Option<String>), ApiError> {
    let anonymous = !headers.contains_key(AUTHORIZATION);
    let credentials_required = || ApiError::unauthorized("Git credentials required");
    let viewer_user_id = if anonymous {
        None
    } else {
        Some(git_read_scope_user(state, headers).await?.id)
    };
    let Some(source) = state
        .metadata
        .repositories()
        .git_read_source(owner, repo_name, viewer_user_id.as_deref())
        .await?
    else {
        return Err(if anonymous {
            credentials_required()
        } else {
            repo_not_found(owner, repo_name)
        });
    };
    let context = &source.context;
    if context.record.lifecycle_state != RepoLifecycleState::Ready {
        return Err(if anonymous {
            credentials_required()
        } else if context.access.actor == RepositoryActor::Owner {
            ApiError::forbidden(AWAITING_FIRST_PUSH_GIT_ERROR)
        } else {
            repo_not_found(owner, repo_name)
        });
    }
    let readable = context.can_read(source.public_files_visible) && context.can_read_view(view);
    match (readable, anonymous) {
        (true, _) => Ok((source, viewer_user_id)),
        (false, true) => Err(credentials_required()),
        (false, false) => Err(ApiError::not_found(format!(
            "Git view {view} of {owner}/{repo_name} not found"
        ))),
    }
}

fn repo_not_found(owner: &str, repo_name: &str) -> ApiError {
    ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
}

pub(crate) async fn git_upload_pack_repo_for_request(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    view: &ViewId,
) -> Result<GitRepoHandle, ApiError> {
    let (source, viewer_user_id) =
        authorized_git_read(state, headers, owner, repo_name, view).await?;
    let repo_id = source.context.record.id.clone();
    let access = source.context.access.clone();
    let views = source.context.views.clone();
    let git = RepositoryGit::of_read_source(source);
    let private_view = view == views.full();
    let base_repo = match git.git_head.as_ref() {
        Some(head) if private_view => {
            state
                .repository_engine
                .materialize_repository(state, &git.incarnation, head, &git.git_pack_spans)
                .await?
        }
        _ => git.view_repo(state, &views, view).await?,
    };
    let mut requests = Vec::new();
    for (request, is_invitee) in state
        .metadata
        .requests()
        .requests_with_invitee_status(&repo_id, viewer_user_id.as_deref())
        .await?
    {
        let decision = request_policy(
            &request,
            RequestViewer::new(access.clone(), viewer_user_id.as_deref(), is_invitee),
        );
        if decision.exact_visible && views.may_read(view, &request.view) {
            requests.push(request);
        }
    }
    requests.sort_by(|left, right| left.name.cmp(&right.name));
    let public_base_repo = if private_view
        && requests
            .iter()
            .any(|request| request.view == ViewId::public() && request.git_snapshot.is_none())
    {
        let public_view = views
            .anyone()
            .ok_or_else(|| ApiError::not_found("public Git view not found"))?;
        Some(git.view_repo(state, &views, public_view).await?)
    } else {
        None
    };
    git_read_view_repo(
        state,
        &git.incarnation,
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
    let read_view_prefix = repository_storage_key(incarnation);
    let read_view_name = format!("read-view-{read_view_prefix}-{cache_key}");
    let repo_path = cache_root.join(format!("{read_view_name}.git"));
    let repo_path_for_ready = repo_path.clone();
    let is_ready = move || repo_path_for_ready.join("objects").is_dir();
    let state_for_build = state.clone();
    let base_repo_for_build = base_repo.share()?;
    let public_base_repo_for_build = public_base_repo
        .as_ref()
        .map(GitRepoHandle::share)
        .transpose()?;
    let requests_for_build = requests.to_vec();
    let cache_root_for_build = cache_root.clone();
    let repo_path_for_build = repo_path.clone();
    let repository_id = incarnation.repository_id().to_string();
    let read_view = state.repository_engine.materialize_derived(
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
    .await?;
    let read_view = read_view.with_dependency(base_repo);
    Ok(match public_base_repo {
        Some(public_base_repo) => read_view.with_dependency(public_base_repo),
        None => read_view,
    })
}

pub(crate) async fn git_upload_pack_response(
    repo: GitRepoHandle,
    request: &[u8],
    timeout: Duration,
    permit: RuntimePermit,
) -> Result<Response, ApiError> {
    let repo_path = repo.as_ref().to_path_buf();
    let mut request = request.to_vec();
    if repo_path.join("objects/info/alternates").is_file() {
        let validation_repo = repo_path.clone();
        request = tokio::task::spawn_blocking(move || {
            prepare_upload_pack_request(&validation_repo, &request)
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("Git upload-pack validation task failed: {error}"))
        })??;
    }
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let work = async move {
        let error_sender = sender.clone();
        let blocking_span = tracing::Span::current();
        let result = tokio::task::spawn_blocking(move || {
            let _entered = blocking_span.enter();
            let _permit = permit;
            let _repo = repo;
            let deadline = Instant::now() + timeout;
            let mut command = git_upload_pack_command();
            command.arg("--stateless-rpc").arg(repo_path);
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

fn prepare_upload_pack_request(repo_path: &FsPath, request: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut required = BTreeSet::new();
    let mut haves = BTreeSet::new();
    let mut have_packets = Vec::new();
    let mut offset = 0;
    while offset < request.len() {
        let packet_start = offset;
        let header = request
            .get(offset..offset + 4)
            .ok_or_else(|| ApiError::bad_request("malformed Git upload-pack packet"))?;
        let length = std::str::from_utf8(header)
            .ok()
            .and_then(|header| usize::from_str_radix(header, 16).ok())
            .ok_or_else(|| ApiError::bad_request("malformed Git upload-pack packet"))?;
        offset += 4;
        if length <= 2 {
            continue;
        }
        if length < 4 {
            return Err(ApiError::bad_request("malformed Git upload-pack packet"));
        }
        let payload = request
            .get(offset..offset + length - 4)
            .ok_or_else(|| ApiError::bad_request("malformed Git upload-pack packet"))?;
        offset += length - 4;
        let required_object = payload
            .strip_prefix(b"want ")
            .or_else(|| payload.strip_prefix(b"shallow "));
        if let Some(object) = required_object.or_else(|| payload.strip_prefix(b"have ")) {
            let oid = object
                .split(|byte| matches!(byte, b' ' | b'\n' | 0))
                .next()
                .unwrap_or_default();
            if oid.len() != 40 || !oid.iter().all(u8::is_ascii_hexdigit) {
                return Err(ApiError::bad_request("malformed Git upload-pack object ID"));
            }
            if required_object.is_some() {
                required.insert(oid.to_ascii_lowercase());
            } else {
                let oid = oid.to_ascii_lowercase();
                haves.insert(oid.clone());
                have_packets.push((packet_start, offset, oid));
            }
        }
    }
    if required.is_empty() && haves.is_empty() {
        return Ok(request.to_vec());
    }
    let tips = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo_path)
            .args(["for-each-ref", "--format=%(objectname)"]),
        None,
    )?;
    for tip in tips.split(|byte| *byte == b'\n') {
        required.remove(tip);
        haves.remove(tip);
    }
    if required.is_empty() && haves.is_empty() {
        return Ok(request.to_vec());
    }
    let reachable = run_with_stdout(
        Command::new("git").arg("--git-dir").arg(repo_path).args([
            "rev-list",
            "--objects",
            "--all",
        ]),
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "checking Git upload-pack object reachability",
        move |stdout, _cancellation| {
            for line in BufReader::new(stdout).split(b'\n') {
                let line = line?;
                let oid = line.split(|byte| *byte == b' ').next().unwrap_or_default();
                required.remove(oid);
                haves.remove(oid);
            }
            Ok::<_, std::io::Error>((required, haves))
        },
    )
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    if !reachable.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "checking Git upload-pack object reachability: {}",
            truncated_git_stderr(&reachable.stderr)
        )));
    }
    let (required, haves) = reachable.value;
    if !required.is_empty() {
        return Err(ApiError::bad_request(
            "Git object is outside the visible refs",
        ));
    }
    let mut filtered = Vec::with_capacity(request.len());
    let mut copied_through = 0;
    for (packet_start, packet_end, oid) in have_packets {
        if haves.contains(&oid) {
            filtered.extend_from_slice(&request[copied_through..packet_start]);
            copied_through = packet_end;
        }
    }
    filtered.extend_from_slice(&request[copied_through..]);
    Ok(filtered)
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
        git_upload_pack_command()
            .args(["--stateless-rpc", "--advertise-refs"])
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

fn git_upload_pack_command() -> Command {
    let mut command = Command::new("git");
    command.args([
        "-c",
        "uploadpack.allowAnySHA1InWant=false",
        "-c",
        "uploadpack.allowReachableSHA1InWant=false",
        "-c",
        "uploadpack.allowTipSHA1InWant=false",
        "upload-pack",
    ]);
    command
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
