mod reconciliation;
#[cfg(test)]
mod tests;

use crate::{config::non_empty_env, error::ApiError, persistence::unix_now, state::AppState};
use scope_api_contract::{
    AttemptCacheKey, CacheRestoreSource, CommitAttemptCacheUploadRequest,
    PrepareAttemptCacheUploadRequest, PrepareAttemptCacheUploadResponse,
    RestoreAttemptCacheResponse,
};
use scope_domain::runs::cache::identity::CacheIdentity;
use scope_postgres::db::{
    CacheAttemptAccess, CacheCommitResult, CachePrepareResult, CacheRestoreKind, CacheUploadRecord,
};
use scope_storage::{S3Backend, S3Presigner, S3Settings};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const SIGNED_URL_TTL_SECONDS: u32 = 15 * 60;
const CACHE_BACKEND_ENV: &str = "SCOPE_CACHE_BACKEND";
const CACHE_BUCKET_ENV_PREFIX: &str = "SCOPE_CACHE_BUCKET";

#[derive(Clone)]
pub(crate) struct RunCacheStorage {
    object_store: Arc<S3Backend>,
    presigner: Arc<S3Presigner>,
    http: reqwest::Client,
    backend: Arc<str>,
}

impl RunCacheStorage {
    pub(crate) fn from_env() -> anyhow::Result<Self> {
        let backend = non_empty_env(CACHE_BACKEND_ENV)
            .ok_or_else(|| anyhow::anyhow!("{CACHE_BACKEND_ENV} is required"))?;
        Self::new(S3Settings::from_env(CACHE_BUCKET_ENV_PREFIX)?, backend)
    }

    fn new(settings: S3Settings, backend: String) -> anyhow::Result<Self> {
        if backend.len() > 64
            || backend.starts_with('-')
            || backend.ends_with('-')
            || backend.contains("--")
            || !backend
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            anyhow::bail!(
                "{CACHE_BACKEND_ENV} must contain lowercase letters, digits, or single hyphens"
            );
        }
        Ok(Self {
            object_store: Arc::new(S3Backend::new(settings.clone())?),
            presigner: Arc::new(S3Presigner::new(&settings)),
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .build()?,
            backend: Arc::from(backend),
        })
    }

    #[cfg(any(test, feature = "local-dev"))]
    pub(crate) fn unreachable() -> Self {
        let mut settings = S3Settings::new(
            "http://127.0.0.1:1".to_string(),
            "scope-run-cache".to_string(),
            "us-east-1".to_string(),
            "local".to_string(),
            "local".to_string(),
        );
        settings.force_path_style = true;
        Self::new(settings, "local".to_string()).expect("local run cache settings are valid")
    }
}

struct AuthorizedCache {
    repository_id: String,
    exact_digest: String,
    compatibility_group_digest: String,
    signed_url_ttl: u32,
}

async fn authorize(
    state: &AppState,
    attempt_id: &str,
    token_hash: &str,
    cache: &AttemptCacheKey,
    now_unix: u64,
) -> Result<AuthorizedCache, ApiError> {
    let CacheAttemptAccess {
        workflow,
        job,
        lease_expires_at_unix,
    } = state
        .metadata
        .runs()
        .authorize_cache_access(attempt_id, token_hash, now_unix)
        .await?;
    let identity = CacheIdentity::for_job(
        &workflow,
        &job,
        &cache.cache_name,
        cache.compatibility_inputs_digest.as_str(),
        cache.exact_inputs_digest.as_str(),
    )?;
    let remaining = lease_expires_at_unix
        .checked_sub(now_unix)
        .filter(|remaining| *remaining > 0)
        .ok_or_else(|| ApiError::unauthorized("run attempt lease has expired"))?;
    Ok(AuthorizedCache {
        repository_id: workflow.repository_id().to_string(),
        exact_digest: identity.exact_digest(),
        compatibility_group_digest: identity.compatibility_group_digest(),
        signed_url_ttl: signed_url_ttl(remaining),
    })
}

fn signed_url_ttl(remaining_lease_seconds: u64) -> u32 {
    u32::try_from(remaining_lease_seconds.min(u64::from(SIGNED_URL_TTL_SECONDS)))
        .expect("signed URL TTL is capped to a u32 constant")
}

pub(crate) async fn restore(
    state: &AppState,
    attempt_id: &str,
    token_hash: &str,
    cache: &AttemptCacheKey,
) -> Result<RestoreAttemptCacheResponse, ApiError> {
    let now = unix_now()?;
    let authorized = authorize(state, attempt_id, token_hash, cache, now).await?;
    let Some(restored) = state
        .metadata
        .caches()
        .restore(
            &authorized.repository_id,
            &authorized.exact_digest,
            &authorized.compatibility_group_digest,
            now,
        )
        .await?
    else {
        return Ok(RestoreAttemptCacheResponse::Miss);
    };
    let storage = &state.run_caches;
    if restored.object.storage_backend != storage.backend.as_ref() {
        return Err(ApiError::internal_message(
            "cache reference targets a different storage backend",
        ));
    }
    let download_url = storage
        .presigner
        .presign(
            "GET",
            &restored.object.object_key,
            authorized.signed_url_ttl,
        )
        .map_err(ApiError::internal)?;
    Ok(RestoreAttemptCacheResponse::Hit {
        source: match restored.source {
            CacheRestoreKind::Exact => CacheRestoreSource::Exact,
            CacheRestoreKind::Compatible => CacheRestoreSource::Compatible,
        },
        object_digest: restored.object.checksum_sha256,
        size_bytes: restored.object.size_bytes,
        download_url,
        expires_at_unix: now + u64::from(authorized.signed_url_ttl),
    })
}

pub(crate) async fn prepare_upload(
    state: &AppState,
    attempt_id: &str,
    token_hash: &str,
    request: &PrepareAttemptCacheUploadRequest,
) -> Result<PrepareAttemptCacheUploadResponse, ApiError> {
    let now = unix_now()?;
    let authorized = authorize(state, attempt_id, token_hash, &request.cache, now).await?;
    let storage = &state.run_caches;
    let prepared = state
        .metadata
        .caches()
        .prepare_upload(
            &authorized.repository_id,
            &authorized.exact_digest,
            &authorized.compatibility_group_digest,
            &request.object_digest,
            request.size_bytes,
            &storage.backend,
            &random_upload_id()?,
            now,
        )
        .await?;
    match prepared {
        CachePrepareResult::UseObject {
            object,
            expires_at_unix,
        } => Ok(PrepareAttemptCacheUploadResponse::UseObject {
            object_digest: object.checksum_sha256,
            expires_at_unix,
        }),
        CachePrepareResult::Upload(upload) => {
            let signed = storage
                .presigner
                .presign_checksum_bound_put(
                    &upload.object_key,
                    authorized.signed_url_ttl,
                    &upload.checksum_sha256,
                    upload.size_bytes,
                )
                .map_err(ApiError::internal)?;
            Ok(PrepareAttemptCacheUploadResponse::Upload {
                lease_id: upload.upload_id,
                upload_url: signed.url,
                upload_headers: BTreeMap::from_iter(signed.headers),
                expires_at_unix: upload.expires_at_unix,
            })
        }
    }
}

pub(crate) async fn commit_upload(
    state: &AppState,
    attempt_id: &str,
    token_hash: &str,
    request: &CommitAttemptCacheUploadRequest,
) -> Result<(), ApiError> {
    let now = unix_now()?;
    let authorized = authorize(state, attempt_id, token_hash, &request.cache, now).await?;
    let storage = &state.run_caches;
    let upload = state.metadata.caches().upload(&request.lease_id).await?;
    if upload.repository_id != authorized.repository_id
        || upload.identity_digest != authorized.exact_digest
        || upload.compatibility_group_digest != authorized.compatibility_group_digest
        || upload.storage_backend != storage.backend.as_ref()
        || upload.checksum_sha256 != request.object_digest
        || upload.size_bytes != request.size_bytes
    {
        return Err(ApiError::forbidden(
            "cache upload does not belong to this attempt or content claim",
        ));
    }
    verify_uploaded_object(storage, &upload).await?;
    match state
        .metadata
        .caches()
        .commit_upload(&request.lease_id, now)
        .await?
    {
        CacheCommitResult::Committed { .. } | CacheCommitResult::AlreadyCommitted { .. } => Ok(()),
        CacheCommitResult::Stale => Err(ApiError::conflict("cache upload lease is stale")),
    }
}

async fn verify_uploaded_object(
    storage: &RunCacheStorage,
    upload: &CacheUploadRecord,
) -> Result<(), ApiError> {
    let url = storage
        .presigner
        .presign("HEAD", &upload.object_key, SIGNED_URL_TTL_SECONDS)
        .map_err(ApiError::internal)?;
    let response = scope_service_runtime::outbound_http::send_traced(
        storage.http.head(url).timeout(Duration::from_secs(30)),
        "/{object_key}",
    )
    .await
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    if !response.status().is_success() {
        return Err(ApiError::conflict(format!(
            "uploaded cache object is unavailable ({})",
            response.status()
        )));
    }
    let uploaded_size = response
        .headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if uploaded_size != Some(upload.size_bytes) {
        return Err(ApiError::conflict(format!(
            "uploaded cache object size does not match its lease (expected {}, received {uploaded_size:?})",
            upload.size_bytes
        )));
    }
    let checksum = response
        .headers()
        .get("x-amz-meta-scope-sha256")
        .and_then(|value| value.to_str().ok());
    if checksum != Some(upload.checksum_sha256.as_str()) {
        return Err(ApiError::conflict(
            "uploaded cache object checksum metadata does not match its lease",
        ));
    }
    Ok(())
}

fn random_upload_id() -> Result<String, ApiError> {
    let mut bytes = [0_u8; 24];
    getrandom::fill(&mut bytes).map_err(|error| ApiError::internal_message(error.to_string()))?;
    Ok(format!("cache_upload_{}", hex::encode(bytes)))
}
