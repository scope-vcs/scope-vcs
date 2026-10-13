use super::{
    archive::create_archive,
    types::{PreparedCache, elapsed_ms},
};
use crate::api::RuntimeClient;
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheFinalizationReport, CacheFinalState as WireCacheFinalState,
    CommitAttemptCacheUploadRequest, PrepareAttemptCacheUploadRequest,
    PrepareAttemptCacheUploadResponse, ReportAttemptCacheFinalizationsRequest,
};
use std::time::Instant;

pub(crate) fn save_caches(client: &RuntimeClient, caches: &[PreparedCache]) {
    let mut reports = Vec::new();
    for cache in caches {
        let started = Instant::now();
        match save_cache(client, cache) {
            Ok(()) => reports.push(AttemptCacheFinalizationReport {
                identity_digest: cache.exact_digest.clone(),
                final_state: WireCacheFinalState::Ready,
                finalize_ms: elapsed_ms(started),
            }),
            Err(error) => eprintln!(
                "runtime cache finalization skipped for {}: {error:#}",
                cache.exact_digest
            ),
        }
    }
    if !reports.is_empty()
        && let Err(error) = client
            .report_cache_finalizations(&ReportAttemptCacheFinalizationsRequest { caches: reports })
    {
        eprintln!("runtime cache finalization reporting skipped: {error:#}");
    }
}

pub(super) fn save_cache(client: &RuntimeClient, cache: &PreparedCache) -> anyhow::Result<()> {
    if cache.exact_hit {
        return Ok(());
    }
    let temp = tempfile::NamedTempFile::new().context("create cache upload file")?;
    let (size_bytes, checksum_sha256) =
        create_archive(&cache.path, temp.path(), cache.sources.as_deref())?;
    let session = client.prepare_cache_upload(&PrepareAttemptCacheUploadRequest {
        cache: cache.key.clone(),
        object_digest: checksum_sha256.clone(),
        size_bytes,
    })?;
    if let PrepareAttemptCacheUploadResponse::Upload {
        lease_id,
        upload_url,
        upload_headers,
        ..
    } = session
    {
        client.upload_cache(&upload_url, &upload_headers, temp.path())?;
        client.commit_cache_upload(&CommitAttemptCacheUploadRequest {
            cache: cache.key.clone(),
            lease_id,
            object_digest: checksum_sha256,
            size_bytes,
        })?;
    }
    Ok(())
}
