pub(crate) mod cache;
pub(crate) mod command;
pub(crate) mod content;
mod context;
mod credentials;
pub(crate) mod import;
pub(crate) mod projection_repo;
pub(crate) mod repository_engine;
pub(crate) mod request_ref_public_safety;
pub(crate) mod request_refs;
pub(crate) mod restore;
pub(crate) mod run_source;
pub(crate) mod storage;
pub(crate) mod upload;

pub(crate) use context::GitContext;

pub(crate) use credentials::*;

use crate::{
    config::*,
    error::ApiError,
    git::{storage::*, upload::*},
    state::AppState,
    use_cases::git_receive::{self, ReceivePackAccess},
};
use axum::{
    Json,
    body::{Body, Bytes, to_bytes},
    extract::{Path, Query, Request, State},
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    ops::Deref,
    path::{Path as FsPath, PathBuf},
    time::Instant,
};

struct TemporaryRepository(PathBuf);

/// Body encodings accepted on Git smart-HTTP requests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GitRequestEncoding {
    Identity,
    Gzip,
}

fn git_request_encoding(headers: &HeaderMap) -> Result<GitRequestEncoding, ApiError> {
    let mut encodings = headers.get_all(CONTENT_ENCODING).iter();
    let Some(encoding) = encodings.next() else {
        return Ok(GitRequestEncoding::Identity);
    };
    if encodings.next().is_some() {
        return Err(ApiError::bad_request(
            "multiple Git content-encoding headers are unsupported",
        ));
    }
    let encoding = encoding
        .to_str()
        .map_err(|_| ApiError::bad_request("invalid Git content-encoding header"))?
        .trim();
    if encoding.is_empty() || encoding.eq_ignore_ascii_case("identity") {
        return Ok(GitRequestEncoding::Identity);
    }
    if encoding.eq_ignore_ascii_case("gzip") {
        return Ok(GitRequestEncoding::Gzip);
    }
    Err(ApiError::bad_request(format!(
        "unsupported Git content-encoding {encoding}"
    )))
}

enum ReceivePackBody {
    Buffered(Vec<u8>),
    Streaming {
        body: Body,
        content_length: Option<u64>,
    },
}

impl Deref for TemporaryRepository {
    type Target = FsPath;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for TemporaryRepository {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct GitInfoRefsQuery {
    pub(crate) service: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GitRemoteMode {
    Public,
    Permissioned,
}

impl GitRemoteMode {
    fn parse(mode: &str) -> Result<Self, ApiError> {
        match mode {
            "public" => Ok(Self::Public),
            "permissioned" => Ok(Self::Permissioned),
            _ => Err(ApiError::not_found(format!(
                "Git remote mode {mode} not found"
            ))),
        }
    }
}

const PUSH_INTENT_HEADER: &str = "x-scope-push-intent";

pub(crate) fn git_error_response(error: ApiError) -> Response {
    if error.status() == StatusCode::UNAUTHORIZED {
        let mut response = error.into_response();
        response.headers_mut().insert(
            WWW_AUTHENTICATE,
            HeaderValue::from_static("Basic realm=\"Scope Git\""),
        );
        return response;
    }
    error.into_response()
}

pub(crate) async fn git_info_refs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((mode, org, repo)): Path<(String, String, String)>,
    Query(query): Query<GitInfoRefsQuery>,
) -> Response {
    let mode = match GitRemoteMode::parse(&mode) {
        Ok(mode) => mode,
        Err(error) => return git_error_response(error),
    };
    match query.service.as_deref() {
        Some(GIT_RECEIVE_PACK) if mode == GitRemoteMode::Public => git_error_response(
            ApiError::forbidden("public Git remote cannot receive pushes"),
        ),
        Some(GIT_RECEIVE_PACK) => {
            let (authorization, push_intent) =
                match receive_pack_credentials(&state, &headers).await {
                    Ok(credentials) => credentials,
                    Err(error) => return git_error_response(error),
                };
            let access = match git_receive::authorize(
                &state,
                &org,
                &repo,
                authorization,
                push_intent.as_deref(),
            )
            .await
            {
                Ok(access) => access,
                Err(error) => return git_error_response(error),
            };
            let _permit = match state.runtime_budgets.try_receive_pack() {
                Ok(permit) => permit,
                Err(error) => return git_error_response(error),
            };
            match handle_git_receive_pack(&state, &org, &repo, "GET", Vec::new(), None, access)
                .await
            {
                Ok(response) => response,
                Err(error) => git_error_response(error),
            }
        }
        Some(GIT_UPLOAD_PACK) => {
            let _permit = match state.runtime_budgets.try_upload_pack() {
                Ok(permit) => permit,
                Err(error) => return git_advertisement_error(error.into_public_message()),
            };
            match git_upload_pack_repo_for_request(&state, &headers, &org, &repo, mode).await {
                Ok(repo_path) => git_upload_pack_advertisement(
                    &repo_path,
                    state.runtime_budgets.git_command_timeout(),
                ),
                Err(error) if error.status() == StatusCode::UNAUTHORIZED => {
                    git_error_response(error)
                }
                Err(error) => git_advertisement_error(error.into_public_message()),
            }
        }
        Some(service) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!("unsupported Git service {service}")
            })),
        )
            .into_response(),
        None => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": "missing Git service"
            })),
        )
            .into_response(),
    }
}

pub(crate) async fn git_receive_pack(
    State(state): State<AppState>,
    Path((mode, org, repo)): Path<(String, String, String)>,
    request: Request,
) -> Response {
    let mode = match GitRemoteMode::parse(&mode) {
        Ok(mode) => mode,
        Err(error) => return git_error_response(error),
    };
    if mode == GitRemoteMode::Public {
        return git_error_response(ApiError::forbidden(
            "public Git remote cannot receive pushes",
        ));
    }
    let headers = request.headers().clone();
    let (authorization, push_intent) = match receive_pack_credentials(&state, &headers).await {
        Ok(credentials) => credentials,
        Err(error) => return git_error_response(error),
    };
    let access =
        match git_receive::authorize(&state, &org, &repo, authorization, push_intent.as_deref())
            .await
        {
            Ok(access) => access,
            Err(error) => return git_error_response(error),
        };
    let _permit = match state.runtime_budgets.try_receive_pack() {
        Ok(permit) => permit,
        Err(error) => return git_error_response(error),
    };

    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let encoding = match git_request_encoding(&headers) {
        Ok(encoding) => encoding,
        Err(error) => return git_error_response(error),
    };
    let request_body = request.into_body();
    let body = match encoding {
        GitRequestEncoding::Gzip => {
            let buffered = match to_bytes(request_body, MAX_RECEIVE_PACK_BYTES).await {
                Ok(body) => body,
                Err(error) => {
                    return git_error_response(ApiError::payload_too_large(format!(
                        "git receive-pack body is too large: {error}"
                    )));
                }
            };
            match decode_git_request_body(&headers, buffered, MAX_RECEIVE_PACK_BYTES) {
                Ok(body) => ReceivePackBody::Buffered(body),
                Err(error) => return git_error_response(error),
            }
        }
        GitRequestEncoding::Identity => {
            let content_length = headers
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok());
            ReceivePackBody::Streaming {
                body: request_body,
                content_length,
            }
        }
    };

    match handle_git_receive_pack_body(&state, &org, &repo, "POST", body, content_type, access)
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(
                owner = org,
                repo,
                status = %error.status(),
                message = error.operator_diagnostic(),
                "git receive-pack failed"
            );
            git_error_response(error)
        }
    }
}

pub(crate) async fn git_upload_pack_rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((mode, org, repo_name)): Path<(String, String, String)>,
    request: Request,
) -> Response {
    let mode = match GitRemoteMode::parse(&mode) {
        Ok(mode) => mode,
        Err(error) => return git_upload_pack_error(error.into_public_message()),
    };
    let permit = match state.runtime_budgets.try_upload_pack() {
        Ok(permit) => permit,
        Err(error) => return git_upload_pack_error(error.into_public_message()),
    };
    let repo_path =
        match git_upload_pack_repo_for_request(&state, &headers, &org, &repo_name, mode).await {
            Ok(repo_path) => repo_path,
            Err(error) => return git_upload_pack_error(error.into_public_message()),
        };
    let body = match to_bytes(request.into_body(), MAX_UPLOAD_PACK_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return git_upload_pack_error(format!("git upload-pack body is too large: {error}"));
        }
    };
    let body = match decode_git_request_body(&headers, body, MAX_UPLOAD_PACK_BYTES) {
        Ok(body) => body,
        Err(error) => return git_upload_pack_error(error.into_public_message()),
    };

    match git_upload_pack_response(
        repo_path,
        &body,
        state.runtime_budgets.git_command_timeout(),
        permit,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => git_upload_pack_error(error.into_public_message()),
    }
}

pub(crate) fn decode_git_request_body(
    headers: &HeaderMap,
    body: Bytes,
    max_bytes: usize,
) -> Result<Vec<u8>, ApiError> {
    if git_request_encoding(headers)? == GitRequestEncoding::Identity {
        return Ok(body.to_vec());
    }

    let mut decoded = Vec::new();
    GzDecoder::new(body.as_ref())
        .take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut decoded)
        .map_err(|error| {
            ApiError::bad_request(format!("invalid gzip Git request body: {error}"))
        })?;
    if decoded.len() > max_bytes {
        return Err(ApiError::payload_too_large(
            "git request body is too large after decompression",
        ));
    }
    Ok(decoded)
}

pub(crate) async fn receive_pack_credentials(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(ReceivePackAuthorization, Option<String>), ApiError> {
    Ok((
        receive_pack_authorization(state, headers).await?,
        optional_push_intent_from_headers(headers)?,
    ))
}
fn optional_push_intent_from_headers(headers: &HeaderMap) -> Result<Option<String>, ApiError> {
    let Some(value) = headers.get(PUSH_INTENT_HEADER) else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| ApiError::forbidden("valid Scope push intent required"))?
        .trim();
    if value.is_empty() {
        Err(ApiError::forbidden("valid Scope push intent required"))
    } else {
        Ok(Some(value.to_string()))
    }
}

pub(crate) async fn handle_git_receive_pack(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    method: &str,
    body: Vec<u8>,
    content_type: Option<String>,
    access: ReceivePackAccess,
) -> Result<Response, ApiError> {
    let preparation = git_receive::prepare(state, owner, repo_name, access, true).await?;
    let remote_user = preparation.access.author_id().to_string();
    let staging_repo = TemporaryRepository(preparation.staging_repo);
    let cgi = git_http_backend(
        &staging_repo,
        method,
        "info/refs",
        "service=git-receive-pack",
        body,
        content_type,
        &remote_user,
    )?;
    Ok(cgi.into_response())
}

async fn handle_git_receive_pack_body(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    method: &str,
    body: ReceivePackBody,
    content_type: Option<String>,
    access: ReceivePackAccess,
) -> Result<Response, ApiError> {
    let preparation = git_receive::prepare(state, owner, repo_name, access, false).await?;
    let remote_user = preparation.access.author_id().to_string();
    let staging_repo = TemporaryRepository(preparation.staging_repo.clone());
    let receive_started_at = Instant::now();
    let cgi = match body {
        ReceivePackBody::Buffered(body) => git_http_backend(
            &staging_repo,
            method,
            "git-receive-pack",
            "",
            body,
            content_type,
            &remote_user,
        )?,
        ReceivePackBody::Streaming {
            body,
            content_length,
        } => {
            git_http_backend_streaming(
                &staging_repo,
                "git-receive-pack",
                body,
                content_length,
                MAX_RECEIVE_PACK_BYTES,
                content_type,
                &remote_user,
            )
            .await?
        }
    };
    let receive_elapsed = receive_started_at.elapsed();
    if cgi.status.is_success()
        && let Err(error) = git_receive::complete(
            state,
            owner,
            repo_name,
            &staging_repo,
            preparation,
            receive_elapsed,
        )
        .await
    {
        tracing::warn!(
            owner,
            repo = repo_name,
            status = %error.status(),
            message = error.operator_diagnostic(),
            "git receive-pack failed"
        );
        // Git has negotiated the response framing. Replace its provisional success
        // with a fatal protocol error, since HTTP errors hide the reason from Git.
        let prefix = if matches!(cgi.body.get(4), Some(1..=3)) {
            "\u{3}"
        } else {
            "ERR "
        };
        return Ok(git_response(
            "application/x-git-receive-pack-result",
            pkt_line(format!("{prefix}{}\n", error.into_public_message()).as_bytes()),
        ));
    }
    Ok(cgi.into_response())
}
