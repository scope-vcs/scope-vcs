use crate::{
    error::ApiError,
    git::{
        http_backend::{CgiResponse, git_http_backend_streaming},
        request_refs::{DeferredRequestObjects, receive_pack_refs},
    },
    state::AppState,
};
use axum::body::{Body, Bytes};
use futures_util::{StreamExt as _, stream};
use std::{
    path::{Path as FsPath, PathBuf},
    time::Instant,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

const SPOOL_CHUNK_BYTES: usize = 64 * 1024;

pub(super) struct ReceivePackRun<'a> {
    pub(super) staging_repo: &'a FsPath,
    pub(super) content_length: Option<u64>,
    pub(super) max_bytes: usize,
    pub(super) content_type: Option<String>,
    pub(super) remote_user: &'a str,
}

impl ReceivePackRun<'_> {
    async fn run(&self, body: Body) -> Result<CgiResponse, ApiError> {
        git_http_backend_streaming(
            self.staging_repo,
            "git-receive-pack",
            body,
            self.content_length,
            self.max_bytes,
            self.content_type.clone(),
            self.remote_user,
        )
        .await
    }
}

pub(super) struct DeferredObjects<'a> {
    pub(super) requests: &'a DeferredRequestObjects,
    pub(super) refs_before: Option<&'a [(String, String)]>,
}

pub(super) async fn receive_pack(
    state: &AppState,
    run: ReceivePackRun<'_>,
    body: Body,
    deferred: Option<DeferredObjects<'_>>,
) -> Result<CgiResponse, ApiError> {
    let Some(deferred) = deferred else {
        return run.run(body).await;
    };
    let spooled = spool_body(state, run.staging_repo, body, run.max_bytes).await?;
    let cgi = run.run(spooled_body(&spooled).await?).await?;
    if !cgi.status.is_success()
        || !receive_pack_lacked_objects(&cgi.body)
        || !refs_unchanged(run.staging_repo, deferred.refs_before).await?
    {
        return Ok(cgi);
    }
    let started = Instant::now();
    deferred.requests.restore(state, run.staging_repo).await?;
    tracing::info!(
        requests = deferred.requests.len(),
        duration_ms = started.elapsed().as_millis(),
        "restored deferred request objects for a push that needed them"
    );
    run.run(spooled_body(&spooled).await?).await
}

fn receive_pack_lacked_objects(report: &[u8]) -> bool {
    let contains = |needle: &[u8]| report.windows(needle.len()).any(|window| window == needle);
    contains(b"missing necessary objects") || (contains(b"unpack ") && !contains(b"unpack ok"))
}

async fn refs_unchanged(
    staging_repo: &FsPath,
    refs_before: Option<&[(String, String)]>,
) -> Result<bool, ApiError> {
    let Some(refs_before) = refs_before.map(<[_]>::to_vec) else {
        return Ok(false);
    };
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || Ok(receive_pack_refs(&staging_repo)? == refs_before)).await
}

async fn spool_body(
    state: &AppState,
    staging_repo: &FsPath,
    body: Body,
    max_bytes: usize,
) -> Result<tempfile::TempPath, ApiError> {
    let parent = staging_repo
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| ApiError::internal_message("staging repository has no parent"))?;
    let spooled = tempfile::Builder::new()
        .prefix(".receive-pack-")
        .tempfile_in(parent)
        .map_err(ApiError::internal)?;
    let (file, path) = spooled.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let deadline = tokio::time::Instant::now() + state.runtime_budgets.git_command_timeout();
    let mut chunks = body.into_data_stream();
    let mut written = 0_usize;
    loop {
        let Ok(chunk) = tokio::time::timeout_at(deadline, chunks.next()).await else {
            return Err(ApiError::infrastructure_unavailable(
                "git request upload timed out",
            ));
        };
        let Some(chunk) = chunk else {
            break;
        };
        let chunk = chunk.map_err(ApiError::bad_request)?;
        written = written.saturating_add(chunk.len());
        if written > max_bytes {
            return Err(ApiError::payload_too_large(
                "git receive-pack body is too large",
            ));
        }
        file.write_all(&chunk).await.map_err(ApiError::internal)?;
    }
    file.flush().await.map_err(ApiError::internal)?;
    Ok(path)
}

async fn spooled_body(path: &FsPath) -> Result<Body, ApiError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(ApiError::internal)?;
    Ok(Body::from_stream(stream::try_unfold(
        file,
        |mut file| async move {
            let mut buffer = vec![0; SPOOL_CHUNK_BYTES];
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                return Ok::<_, std::io::Error>(None);
            }
            buffer.truncate(read);
            Ok(Some((Bytes::from(buffer), file)))
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::receive_pack_lacked_objects;

    #[test]
    fn missing_objects_are_recognized_in_receive_pack_reports() {
        assert!(receive_pack_lacked_objects(
            b"000eunpack ok\n0030ng refs/heads/topic missing necessary objects\n0000"
        ));
        assert!(receive_pack_lacked_objects(
            b"0029unpack unpack-objects abnormal exit\n0000"
        ));
        assert!(!receive_pack_lacked_objects(
            b"000eunpack ok\n0017ok refs/heads/topic\n0000"
        ));
        assert!(!receive_pack_lacked_objects(
            b"000eunpack ok\n0024ng refs/heads/topic pre-receive hook declined\n0000"
        ));
    }
}
