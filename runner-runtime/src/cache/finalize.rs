use super::{
    archive::create_archive,
    types::{CacheFinalization, CacheFinalizationOutcome, PreparedCache, elapsed_ms},
};
use crate::api::RuntimeClient;
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheFinalizationReport, CacheFinalState as WireCacheFinalState,
    CommitAttemptCacheUploadRequest, PrepareAttemptCacheUploadRequest,
    PrepareAttemptCacheUploadResponse, ReportAttemptCacheFinalizationsRequest,
};
use std::time::Instant;

pub(crate) fn save_caches(
    client: &RuntimeClient,
    caches: &[PreparedCache],
) -> Vec<CacheFinalization> {
    let mut finalizations = Vec::with_capacity(caches.len());
    let mut reports = Vec::new();
    for cache in caches {
        let started = Instant::now();
        let outcome = save_cache(client, cache);
        if matches!(outcome, CacheFinalizationOutcome::Ready) {
            reports.push(AttemptCacheFinalizationReport {
                identity_digest: cache.exact_digest.clone(),
                final_state: WireCacheFinalState::Ready,
                finalize_ms: elapsed_ms(started),
            });
        }
        finalizations.push(CacheFinalization {
            identity_digest: cache.exact_digest.clone(),
            outcome,
        });
    }
    if !reports.is_empty()
        && let Err(error) = client
            .report_cache_finalizations(&ReportAttemptCacheFinalizationsRequest { caches: reports })
    {
        eprintln!("runtime cache finalization reporting skipped: {error:#}");
    }
    finalizations
}
pub(super) fn save_cache(
    client: &RuntimeClient,
    cache: &PreparedCache,
) -> CacheFinalizationOutcome {
    if cache.exact_hit {
        return CacheFinalizationOutcome::Ready;
    }
    let temp = match tempfile::NamedTempFile::new().context("create cache upload file") {
        Ok(temp) => temp,
        Err(error) => return CacheFinalizationOutcome::Skipped(error),
    };
    let (size_bytes, checksum_sha256) =
        match create_archive(&cache.path, temp.path(), cache.sources.as_deref()) {
            Ok(identity) => identity,
            Err(error) => return CacheFinalizationOutcome::Skipped(error),
        };
    let session = match client.prepare_cache_upload(&PrepareAttemptCacheUploadRequest {
        cache: cache.key.clone(),
        object_digest: checksum_sha256.clone(),
        size_bytes,
    }) {
        Ok(session) => session,
        Err(error) => return CacheFinalizationOutcome::Skipped(error),
    };
    match session {
        PrepareAttemptCacheUploadResponse::UseObject { .. } => CacheFinalizationOutcome::Ready,
        PrepareAttemptCacheUploadResponse::Upload {
            lease_id,
            upload_url,
            upload_headers,
            ..
        } => {
            if let Err(error) = client.upload_cache(&upload_url, &upload_headers, temp.path()) {
                return CacheFinalizationOutcome::Skipped(error);
            }
            if let Err(error) = client.commit_cache_upload(&CommitAttemptCacheUploadRequest {
                cache: cache.key.clone(),
                lease_id,
                object_digest: checksum_sha256,
                size_bytes,
            }) {
                return CacheFinalizationOutcome::Skipped(error);
            }
            CacheFinalizationOutcome::Ready
        }
    }
}
