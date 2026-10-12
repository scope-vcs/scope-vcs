use super::{
    archive::{RestoredArchive, extract_archive, reset_cache_directory},
    identity::digest_inputs,
    sources::SourceSnapshot,
    types::{PreparedCache, elapsed_ms},
};
use crate::api::{RuntimeClient, cache_client::CacheDownloadError};
use anyhow::Context as _;
use scope_api_contract::{
    AttemptCacheKey, AttemptCachePreparationReport, CacheColdReason as WireCacheColdReason,
    CachePreparation as WireCachePreparation, CacheRestoreSource,
    ReportAttemptCachePreparationsRequest, RestoreAttemptCacheRequest, RestoreAttemptCacheResponse,
    RunJobResponse,
};
use scope_domain::runs::{
    cache::{
        identity::CacheIdentity,
        observation::{AttemptCachePreparationTiming, CacheColdReason, CachePreparation},
    },
    workflow::identity::{WorkflowIdentity, WorkflowPath},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

pub(crate) fn prepare_caches(
    client: &RuntimeClient,
    job: &RunJobResponse,
    definition: &scope_domain::runs::workflow::definition::WorkflowJob,
    workspace: &Path,
) -> anyhow::Result<Vec<PreparedCache>> {
    let setup_started = Instant::now();
    let workflow = WorkflowIdentity::new(
        job.repository_id.as_str(),
        WorkflowPath::parse(job.workflow_path.clone())?,
    )?;
    let mut identities = Vec::new();
    for cache in definition.caches() {
        let started = Instant::now();
        let compatibility_inputs_digest = digest_inputs(
            cache.compatibility_inputs(),
            definition.environment(),
            &job.git_oid,
        )?;
        let exact_inputs_digest =
            digest_inputs(cache.exact_inputs(), definition.environment(), &job.git_oid)?;
        let identity = CacheIdentity::for_job(
            &workflow,
            definition,
            cache.as_str(),
            compatibility_inputs_digest.as_str(),
            exact_inputs_digest.as_str(),
        )?;
        let key = AttemptCacheKey {
            cache_name: cache.as_str().to_string(),
            compatibility_inputs_digest,
            exact_inputs_digest,
        };
        identities.push((key, identity, elapsed_ms(started)));
    }
    let mut prepared = Vec::new();
    let mut reports = Vec::new();
    let mut restored_archives = Vec::new();
    let mut sources = definition
        .caches()
        .iter()
        .any(|cache| cache.includes_source())
        .then(|| SourceSnapshot::capture(workspace))
        .transpose()?;
    for (cache, (key, identity, key_ms)) in definition.caches().iter().zip(identities) {
        let exact_digest = identity.exact_digest();
        let path = PathBuf::from(cache.mount_path());
        fs::create_dir_all(&path)
            .with_context(|| format!("create cache path {}", path.display()))?;
        let resolved_path = fs::canonicalize(&path)?;
        if resolved_path.starts_with(workspace) || workspace.starts_with(&resolved_path) {
            anyhow::bail!("cache path overlaps the checkout: {}", path.display());
        }
        let source_root = cache.includes_source().then_some(workspace);
        let restore = restore_cache(client, &key, &exact_digest, &path, source_root, key_ms)?;
        let timing = restore.timing;
        reports.push(AttemptCachePreparationReport {
            cache_name: cache.as_str().to_string(),
            identity_digest: exact_digest.clone(),
            preparation: wire_cache_preparation(restore.preparation),
            key_ms: timing.key_ms(),
            metadata_ms: timing.metadata_ms(),
            size_bytes: timing.size_bytes(),
            download_verify_ms: timing.download_verify_ms(),
            sync_ms: timing.sync_ms(),
            extraction_ms: timing.extraction_ms(),
            prepare_ms: timing.prepare_ms(),
        });
        prepared.push(PreparedCache {
            key,
            exact_digest,
            path,
            exact_hit: restore.exact_hit,
            sources: None,
        });
        if let Some(archive) = restore.archive {
            restored_archives.push(archive);
        }
    }
    if let Some(snapshot) = &mut sources {
        snapshot.restore(&restored_archives)?;
    }
    let sources = sources.map(Arc::new);
    for (cache, definition) in prepared.iter_mut().zip(definition.caches()) {
        if definition.includes_source() {
            cache.sources = sources.clone();
        }
    }
    let wall_ms = elapsed_ms(setup_started);
    if let Err(error) = client.report_cache_preparations(&ReportAttemptCachePreparationsRequest {
        authorization_ms: 0,
        wall_ms,
        caches: reports,
    }) {
        eprintln!("runtime cache preparation reporting skipped: {error:#}");
    }
    Ok(prepared)
}
fn restore_cache(
    client: &RuntimeClient,
    key: &AttemptCacheKey,
    exact_digest: &str,
    destination: &Path,
    source_root: Option<&Path>,
    key_ms: u64,
) -> anyhow::Result<CacheRestore> {
    let metadata_started = Instant::now();
    let session = client.restore_cache(&RestoreAttemptCacheRequest { cache: key.clone() });
    let metadata_ms = elapsed_ms(metadata_started);
    let timing = |size_bytes, download_verify_ms, sync_ms, extraction_ms| {
        AttemptCachePreparationTiming::measured(
            key_ms,
            metadata_ms,
            size_bytes,
            download_verify_ms,
            sync_ms,
            extraction_ms,
        )
    };
    let session = match session {
        Ok(session) => session,
        Err(error) => {
            eprintln!("runtime cache restore unavailable for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(
                CacheColdReason::MetadataNotReady,
                timing(0, 0, 0, 0)?,
            ));
        }
    };
    let (source, url, checksum, size) = match session {
        RestoreAttemptCacheResponse::Hit {
            source,
            object_digest,
            size_bytes,
            download_url,
            ..
        } => (source, download_url, object_digest, size_bytes),
        RestoreAttemptCacheResponse::Miss => {
            return Ok(CacheRestore::cold(
                CacheColdReason::MetadataMissing,
                timing(0, 0, 0, 0)?,
            ));
        }
    };
    let temp_dir = match tempfile::tempdir().context("create cache download directory") {
        Ok(temp_dir) => temp_dir,
        Err(error) => {
            eprintln!("runtime cache restore staging failed for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(
                CacheColdReason::MetadataNotReady,
                timing(size, 0, 0, 0)?,
            ));
        }
    };
    let archive = temp_dir.path().join("cache.tar.zst");
    let download = client.download_cache(&url, &archive, size, &checksum);
    if let Err(error) = download.outcome {
        let reason = match error {
            CacheDownloadError::Transport(error) => {
                eprintln!("runtime cache restore transport failed for {exact_digest}: {error:#}");
                CacheColdReason::MetadataNotReady
            }
            CacheDownloadError::Invalid(error) => {
                eprintln!("runtime cache restore rejected for {exact_digest}: {error:#}");
                CacheColdReason::MetadataInvalid
            }
        };
        return Ok(CacheRestore::cold(
            reason,
            timing(size, download.download_verify_ms, download.sync_ms, 0)?,
        ));
    }
    let extraction_started = Instant::now();
    let extraction = extract_archive(&archive, destination, source_root);
    let timing = timing(
        size,
        download.download_verify_ms,
        download.sync_ms,
        elapsed_ms(extraction_started),
    )?;
    let archive = match extraction {
        Ok(archive) => archive,
        Err(error) => {
            reset_cache_directory(destination)?;
            eprintln!("runtime cache restore was corrupt for {exact_digest}: {error:#}");
            return Ok(CacheRestore::cold(CacheColdReason::MetadataInvalid, timing));
        }
    };
    Ok(CacheRestore {
        preparation: match source {
            CacheRestoreSource::Exact => CachePreparation::Exact,
            CacheRestoreSource::Compatible => CachePreparation::Compatible,
        },
        exact_hit: source == CacheRestoreSource::Exact,
        timing,
        archive: Some(archive),
    })
}

struct CacheRestore {
    preparation: CachePreparation,
    exact_hit: bool,
    timing: AttemptCachePreparationTiming,
    archive: Option<RestoredArchive>,
}

impl CacheRestore {
    fn cold(reason: CacheColdReason, timing: AttemptCachePreparationTiming) -> Self {
        Self {
            preparation: CachePreparation::Cold { reason },
            exact_hit: false,
            timing,
            archive: None,
        }
    }
}

fn wire_cache_preparation(preparation: CachePreparation) -> WireCachePreparation {
    match preparation {
        CachePreparation::Exact => WireCachePreparation::Exact,
        CachePreparation::Compatible => WireCachePreparation::Compatible,
        CachePreparation::Cold { reason } => WireCachePreparation::Cold {
            reason: match reason {
                CacheColdReason::MetadataMissing => WireCacheColdReason::MetadataMissing,
                CacheColdReason::MetadataInvalid => WireCacheColdReason::MetadataInvalid,
                CacheColdReason::MetadataNotReady => WireCacheColdReason::MetadataNotReady,
            },
        },
    }
}
