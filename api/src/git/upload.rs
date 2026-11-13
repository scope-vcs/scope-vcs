use crate::{
    config::{AWAITING_FIRST_PUSH_GIT_ERROR, GIT_UPLOAD_PACK},
    error::ApiError,
    git::{
        cache::GitRepoHandle,
        command::{
            git_command_output, git_command_output_with_timeout, prepare_git_subprocess,
            record_git_exit, truncated_git_stderr,
        },
        git_read_scope_user,
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
    repository::RepoLifecycleState, repository::access::RepositoryActor, views::ViewId,
};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_postgres::db::GitReadSource;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader, Read},
    path::Path as FsPath,
    process::Command,
    time::{Duration, Instant},
};
use tracing::Instrument as _;
pub(super) mod fetch_folder;
#[cfg(test)]
pub(crate) use fetch_folder::prepare as git_upload_pack_repo_for_request;
#[cfg(test)]
mod fetch_tests;
mod protocol;
pub(crate) use protocol::GitProtocol;

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
    let repository_readable = context.can_read(source.public_files_visible);
    match (repository_readable, context.can_read_view(view), anonymous) {
        (true, true, _) => Ok((source, viewer_user_id)),
        (_, _, true) => Err(credentials_required()),
        (false, _, false) => Err(repo_not_found(owner, repo_name)),
        (true, false, false) => Err(ApiError::not_found(format!(
            "Git view {view} of {owner}/{repo_name} not found"
        ))),
    }
}

fn repo_not_found(owner: &str, repo_name: &str) -> ApiError {
    ApiError::not_found(format!("repo {owner}/{repo_name} not found"))
}

pub(crate) async fn git_upload_pack_response(
    repo: GitRepoHandle,
    request: &[u8],
    protocol: GitProtocol,
    timeout: Duration,
    permit: RuntimePermit,
) -> Result<Response, ApiError> {
    let repo_path = repo.as_ref().to_path_buf();
    let mut request = request.to_vec();
    if repo_path.join("objects/info/alternates").is_file() {
        let validation_repo = repo_path.clone();
        let parent_span = tracing::Span::current();
        request = tokio::task::spawn_blocking(move || {
            let _entered = parent_span.enter();
            prepare_upload_pack_request(&validation_repo, &request)
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("Git upload-pack validation task failed: {error}"))
        })??;
    }
    let mut command = git_upload_pack_command(protocol);
    command.arg("--stateless-rpc").arg(repo_path);
    let git_span = prepare_git_subprocess(&mut command)?;
    let (sender, receiver) = tokio::sync::mpsc::channel(2);
    let work = async move {
        let error_sender = sender.clone();
        let blocking_span = tracing::Span::current();
        let result = tokio::task::spawn_blocking(move || {
            let _entered = blocking_span.enter();
            let _permit = permit;
            let _repo = repo;
            let deadline = Instant::now() + timeout;
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
        if let Some(message) = stream_error
            && let Err(error) = error_sender.try_send(Err(std::io::Error::other(message)))
        {
            tracing::debug!(%error, "Git upload-pack stream closed before its error was delivered");
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

struct UploadPackPacket<'a> {
    start: usize,
    end: usize,
    payload: &'a [u8],
}

fn upload_pack_packets(request: &[u8]) -> Result<Vec<UploadPackPacket<'_>>, ApiError> {
    let mut packets = Vec::new();
    let mut offset = 0;
    while offset < request.len() {
        let start = offset;
        let header = request
            .get(offset..offset + 4)
            .ok_or_else(|| ApiError::bad_request("malformed Git upload-pack packet"))?;
        let length = pkt_line_length(header)
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
        packets.push(UploadPackPacket {
            start,
            end: offset,
            payload,
        });
    }
    Ok(packets)
}

fn upload_pack_oid(object: &[u8]) -> Result<Vec<u8>, ApiError> {
    let oid = object
        .split(|byte| matches!(byte, b' ' | b'\n' | 0))
        .next()
        .unwrap_or_default();
    if oid.len() != 40 || !oid.iter().all(u8::is_ascii_hexdigit) {
        return Err(ApiError::bad_request("malformed Git upload-pack object ID"));
    }
    Ok(oid.to_ascii_lowercase())
}

fn prepare_upload_pack_request(repo_path: &FsPath, request: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut required = BTreeSet::new();
    let mut haves = BTreeSet::new();
    let mut have_packets = Vec::new();
    for packet in upload_pack_packets(request)? {
        let required_object = packet
            .payload
            .strip_prefix(b"want ")
            .or_else(|| packet.payload.strip_prefix(b"shallow "));
        if let Some(object) = required_object.or_else(|| packet.payload.strip_prefix(b"have ")) {
            let oid = upload_pack_oid(object)?;
            if required_object.is_some() {
                required.insert(oid);
            } else {
                haves.insert(oid.clone());
                have_packets.push((packet.start, packet.end, oid));
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
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo_path)
        .args(["rev-list", "--objects", "--all"]);
    let git_span = prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let reachable = run_with_stdout(
        &mut command,
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
    record_git_exit(&git_span, reachable.status);
    if !reachable.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "checking Git upload-pack object reachability: {}",
            truncated_git_stderr(&reachable.stderr)
        )));
    }
    let (required, haves) = reachable.value;
    if !required.is_empty() {
        return Err(ApiError::bad_request(
            "Git object is outside the visible refs; refs may have changed, retry fetch",
        ));
    }
    let mut unacknowledged = have_packets
        .iter()
        .all(|(_, _, oid)| haves.contains(oid))
        .then(|| pkt_line(format!("have {UNKNOWN_HAVE}\n").as_bytes()));
    let mut filtered = Vec::with_capacity(request.len());
    let mut copied_through = 0;
    for (packet_start, packet_end, oid) in have_packets {
        if haves.contains(&oid) {
            filtered.extend_from_slice(&request[copied_through..packet_start]);
            filtered.extend(unacknowledged.take().unwrap_or_default());
            copied_through = packet_end;
        }
    }
    filtered.extend_from_slice(&request[copied_through..]);
    Ok(filtered)
}

const UNKNOWN_HAVE: &str = "0000000000000000000000000000000000000000";

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

pub(crate) fn git_upload_pack_advertisement(
    repo_path: &FsPath,
    protocol: GitProtocol,
    timeout: Duration,
) -> Response {
    match git_command_output_with_timeout(
        git_upload_pack_command(protocol)
            .args(["--stateless-rpc", "--advertise-refs"])
            .arg(repo_path),
        None,
        timeout,
    ) {
        Ok(advertisement) => {
            let mut body = Vec::new();
            if protocol == GitProtocol::V0 {
                body.extend(pkt_line(
                    format!("# service={GIT_UPLOAD_PACK}\n").as_bytes(),
                ));
                body.extend_from_slice(b"0000");
            }
            body.extend(advertisement);
            git_response("application/x-git-upload-pack-advertisement", body)
        }
        Err(error) => git_advertisement_error(error.into_public_message()),
    }
}

fn git_upload_pack_command(protocol: GitProtocol) -> Command {
    let mut command = Command::new("git");
    protocol.configure(&mut command);
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

pub(crate) fn pkt_line_length(header: &[u8]) -> Option<usize> {
    std::str::from_utf8(header)
        .ok()
        .and_then(|header| usize::from_str_radix(header, 16).ok())
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
