use crate::{
    error::ApiError, git::upload::git_command_output_with_timeout, runtime_budgets::RuntimeBudgets,
};
use axum::{body::Body, http::StatusCode, response::Response};
use futures_util::StreamExt;
#[cfg(all(test, target_os = "linux"))]
use std::fs;
use std::{
    path::Path as FsPath,
    process::{Command, Stdio},
    time::Instant,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
};

pub(crate) fn git_http_backend(
    staging_repo: &FsPath,
    method: &str,
    path_suffix: &str,
    query_string: &str,
    body: Vec<u8>,
    content_type: Option<String>,
    remote_user: &str,
) -> Result<CgiResponse, ApiError> {
    let staging_parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repo is missing a parent"))?;
    let repo_name = staging_repo
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("staging repo has invalid path"))?;
    let mut command = Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", staging_parent)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", method)
        .env("PATH_INFO", format!("/{repo_name}/{path_suffix}"))
        .env("QUERY_STRING", query_string)
        .env("REMOTE_USER", remote_user)
        .env("CONTENT_LENGTH", body.len().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let output = git_command_output_with_timeout(
        &mut command,
        Some(body),
        RuntimeBudgets::default_git_command_timeout(),
    )?;
    CgiResponse::parse(output)
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn git_http_backend_streaming(
    staging_repo: &FsPath,
    path_suffix: &str,
    body: Body,
    content_length: Option<u64>,
    max_bytes: usize,
    content_type: Option<String>,
    remote_user: &str,
) -> Result<CgiResponse, ApiError> {
    let receive_started = Instant::now();
    if content_length.is_some_and(|length| length > max_bytes as u64) {
        return Err(ApiError::payload_too_large(
            "git receive-pack body is too large",
        ));
    }
    let staging_parent = staging_repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("staging repo is missing a parent"))?;
    let repo_name = staging_repo
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| ApiError::internal_message("staging repo has invalid path"))?;
    let mut command = tokio::process::Command::new("git");
    command
        .arg("http-backend")
        .env("GIT_PROJECT_ROOT", staging_parent)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", "POST")
        .env("PATH_INFO", format!("/{repo_name}/{path_suffix}"))
        .env("QUERY_STRING", "")
        .env("REMOTE_USER", remote_user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    scope_git_process::configure_process_group(command.as_std_mut());
    if let Some(content_length) = content_length {
        command.env("CONTENT_LENGTH", content_length.to_string());
    }
    if let Some(content_type) = content_type {
        command.env("CONTENT_TYPE", content_type);
    }

    let mut child = command.spawn().map_err(ApiError::internal)?;
    let mut process_group = GitProcessGroupGuard::new(child.id());
    let Some(mut stdin) = child.stdin.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stdin failed",
        ));
    };
    let Some(stdout) = child.stdout.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stdout failed",
        ));
    };
    let Some(stderr) = child.stderr.take() else {
        terminate_and_reap_git_child(&mut child, &mut process_group).await;
        return Err(ApiError::internal_message(
            "opening git http-backend stderr failed",
        ));
    };
    let mut stdout_task = tokio::spawn(read_git_pipe(stdout));
    let mut stderr_task = tokio::spawn(read_git_pipe(stderr));
    let process_timeout = RuntimeBudgets::default_git_command_timeout();
    let process_deadline = tokio::time::Instant::now() + process_timeout;
    let writer = async move {
        let mut stream = body.into_data_stream();
        let mut written = 0usize;
        loop {
            let next = tokio::time::timeout_at(process_deadline, stream.next())
                .await
                .map_err(|_| ApiError::infrastructure_unavailable("git request upload stalled"))?;
            let Some(chunk) = next else {
                break;
            };
            let chunk = chunk.map_err(ApiError::bad_request)?;
            written = written
                .checked_add(chunk.len())
                .ok_or_else(|| ApiError::payload_too_large("git receive-pack body is too large"))?;
            if written > max_bytes {
                return Err(ApiError::payload_too_large(
                    "git receive-pack body is too large",
                ));
            }
            stdin.write_all(&chunk).await.map_err(ApiError::internal)?;
        }
        stdin.shutdown().await.map_err(ApiError::internal)?;
        Ok::<usize, ApiError>(written)
    };
    let request_bytes = match tokio::time::timeout_at(process_deadline, writer).await {
        Ok(Ok(written)) => written,
        Ok(Err(error)) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(error);
        }
        Err(_) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(ApiError::infrastructure_unavailable(
                "git request upload timed out",
            ));
        }
    };
    let output = match tokio::time::timeout_at(
        process_deadline,
        collect_git_http_backend_output(&mut child, &mut stdout_task, &mut stderr_task),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(error);
        }
        Err(_) => {
            stop_git_http_backend(
                &mut child,
                &mut process_group,
                &mut stdout_task,
                &mut stderr_task,
            )
            .await;
            return Err(ApiError::infrastructure_unavailable(
                "git http-backend timed out",
            ));
        }
    };
    process_group.disarm();
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "git http-backend failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    tracing::info!(
        request_bytes,
        receive_ms = receive_started.elapsed().as_millis(),
        "streamed Git receive-pack body"
    );
    CgiResponse::parse(output.stdout)
}

async fn read_git_pipe(mut pipe: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn collect_git_http_backend_output(
    child: &mut tokio::process::Child,
    stdout_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
) -> Result<std::process::Output, ApiError> {
    let stdout = join_git_pipe(stdout_task, "stdout").await?;
    let stderr = join_git_pipe(stderr_task, "stderr").await?;
    let status = child.wait().await.map_err(ApiError::internal)?;
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

async fn join_git_pipe(
    task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    pipe: &str,
) -> Result<Vec<u8>, ApiError> {
    task.await
        .map_err(|_| ApiError::internal_message(format!("git http-backend {pipe} task panicked")))?
        .map_err(ApiError::internal)
}

async fn stop_git_http_backend(
    child: &mut tokio::process::Child,
    process_group: &mut GitProcessGroupGuard,
    stdout_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
    stderr_task: &mut JoinHandle<std::io::Result<Vec<u8>>>,
) {
    terminate_and_reap_git_child(child, process_group).await;
    stdout_task.abort();
    stderr_task.abort();
    let _ = stdout_task.await;
    let _ = stderr_task.await;
}

async fn terminate_and_reap_git_child(
    child: &mut tokio::process::Child,
    process_group: &mut GitProcessGroupGuard,
) {
    process_group.kill();
    let _ = child.kill().await;
    let _ = child.wait().await;
}

struct GitProcessGroupGuard {
    process_id: Option<u32>,
}

impl GitProcessGroupGuard {
    fn new(process_id: Option<u32>) -> Self {
        Self { process_id }
    }

    fn kill(&mut self) {
        if let Some(process_id) = self.process_id.take() {
            scope_git_process::kill_process_group(process_id);
        }
    }

    fn disarm(&mut self) {
        self.process_id = None;
    }
}

impl Drop for GitProcessGroupGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

pub(crate) struct CgiResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl CgiResponse {
    pub(crate) fn parse(output: Vec<u8>) -> Result<Self, ApiError> {
        let header_end = find_header_end(&output).ok_or_else(|| {
            ApiError::infrastructure_unavailable("git http-backend returned no headers")
        })?;
        let (headers, body) = output.split_at(header_end.0);
        let headers = String::from_utf8_lossy(headers);
        let mut status = StatusCode::OK;
        let mut parsed_headers = Vec::new();

        for line in headers
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("Status") {
                let code = value
                    .split_whitespace()
                    .next()
                    .and_then(|code| code.parse::<u16>().ok())
                    .ok_or_else(|| {
                        ApiError::infrastructure_unavailable("invalid git CGI status")
                    })?;
                status = StatusCode::from_u16(code).map_err(ApiError::internal)?;
            } else {
                parsed_headers.push((name.trim().to_string(), value.trim().to_string()));
            }
        }

        Ok(Self {
            status,
            headers: parsed_headers,
            body: body[header_end.1..].to_vec(),
        })
    }

    pub(crate) fn into_response(self) -> Response {
        let mut builder = Response::builder().status(self.status);
        for (name, value) in self.headers {
            builder = builder.header(name, value);
        }
        builder
            .body(Body::from(self.body))
            .expect("git CGI response headers should be valid")
    }
}

fn find_header_end(bytes: &[u8]) -> Option<(usize, usize)> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, 4))
        .or_else(|| {
            bytes
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|index| (index, 2))
        })
}

#[cfg(all(test, target_os = "linux"))]
mod process_tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn terminating_git_child_reaps_child_and_kills_its_process_group() {
        let mut command = tokio::process::Command::new("sh");
        command
            .arg("-c")
            .arg("sleep 30 & printf '%s\\n' $!; wait")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        scope_git_process::configure_process_group(command.as_std_mut());

        let mut child = command.spawn().expect("spawn test process group");
        let stdout = child.stdout.take().expect("test child stdout");
        let mut stdout = BufReader::new(stdout);
        let mut descendant = String::new();
        stdout
            .read_line(&mut descendant)
            .await
            .expect("read descendant pid");
        let descendant = descendant.trim().parse::<u32>().expect("descendant pid");

        let mut process_group = GitProcessGroupGuard::new(child.id());
        terminate_and_reap_git_child(&mut child, &mut process_group).await;

        assert!(child.id().is_none(), "direct child was not reaped");
        let mut descendant_state = None;
        for _ in 0..100 {
            descendant_state = fs::read_to_string(format!("/proc/{descendant}/stat"))
                .ok()
                .and_then(|stat| {
                    let command_end = stat.rfind(')')?;
                    stat[command_end + 2..]
                        .split_whitespace()
                        .next()
                        .map(str::to_string)
                });
            if descendant_state.as_deref().is_none_or(|state| state == "Z") {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(
            descendant_state.as_deref().is_none_or(|state| state == "Z"),
            "descendant survived process-group kill in state {descendant_state:?}"
        );
    }
}
