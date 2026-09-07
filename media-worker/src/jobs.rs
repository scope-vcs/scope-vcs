use crate::{
    codec::{
        CodecDerivative, CodecFailure, CodecFailureKind, CodecPipeline, DerivativeKind, MediaKind,
        ValidatedSource,
    },
    config::WorkerSettings,
    health::WorkerHealth,
    scratch::ScratchSpace,
};
use scope_domain::requests::attachments::{
    RequestAttachmentDerivative, RequestAttachmentDerivativeKind, RequestAttachmentFailure,
    RequestAttachmentFailureCode, RequestAttachmentProcessingLease, RequestAttachmentStoredObject,
};
use scope_media_storage::{
    MAX_CHUNK_BYTES, MediaChunk, MediaObject, MediaStorage, MediaStorageError,
    MediaStorageErrorKind, StagedMediaPart, WriteAttempt,
};
use scope_postgres::db::{
    CompleteRequestAttachmentProcessingCommand, CompletedRequestAttachmentDerivative,
    CompletedRequestMediaManifest, FailRequestAttachmentProcessingCommand, MediaLeaseMutation,
    MetadataStore, RequestMediaChunk, RequestMediaManifest, ValidateRequestAttachmentSourceCommand,
    ValidatedRequestAttachmentSource,
};
use sha2::{Digest, Sha256};
use std::{future::Future, path::Path, time::Duration};
use tokio::io::AsyncReadExt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessingOutcome {
    NoJob,
    Completed,
    Failed,
    LeaseLost,
}

pub async fn run_processing_loop(
    metadata: MetadataStore,
    storage: MediaStorage,
    pipeline: CodecPipeline,
    scratch: ScratchSpace,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    loop {
        if !dependencies_ready(&metadata, &storage, &health).await {
            if wait_or_shutdown(settings.poll_interval).await {
                return Ok(());
            }
            continue;
        }
        let outcome =
            process_next_job(&metadata, &storage, &pipeline, &scratch, &settings, &health).await;
        let should_wait = should_wait_after_poll(&outcome);
        match &outcome {
            Ok(ProcessingOutcome::NoJob) => {}
            Ok(value) => tracing::info!(outcome = ?value, "media processing job finished"),
            Err(error) => tracing::error!(%error, "media processing poll failed"),
        }
        health.mark_processing_poll(crate::unix_now()?);
        if !should_wait {
            continue;
        }
        if wait_or_shutdown(settings.poll_interval).await {
            return Ok(());
        }
    }
}

fn should_wait_after_poll(outcome: &anyhow::Result<ProcessingOutcome>) -> bool {
    matches!(outcome, Ok(ProcessingOutcome::NoJob) | Err(_))
}

async fn process_next_job(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    pipeline: &CodecPipeline,
    scratch: &ScratchSpace,
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> anyhow::Result<ProcessingOutcome> {
    let now = crate::unix_now()?;
    let token = random_id("lease")?;
    let Some(lease) = metadata
        .media()
        .claim_processing_job(&token, now, lease_expiry(now, settings.lease_duration)?)
        .await
        .map_err(db_error)?
    else {
        return Ok(ProcessingOutcome::NoJob);
    };
    tracing::info!(
        attachment_id = %lease.attachment_id,
        attempt = lease.attempt,
        lease_generation = lease.lease_generation,
        "claimed media processing job"
    );
    let _activity = health.processing_activity();
    let work = settle_claim(metadata, storage, pipeline, scratch, settings, &lease);
    match supervise_processing_claim(work, settings.lease_duration, || async {
        let now = crate::unix_now()?;
        renew_processing(metadata, &lease, settings.lease_duration, now).await
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(HeartbeatError::LeaseLost) => Ok(ProcessingOutcome::LeaseLost),
        Err(HeartbeatError::Database(error)) => Err(error),
    }
}

async fn settle_claim(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    pipeline: &CodecPipeline,
    scratch: &ScratchSpace,
    settings: &WorkerSettings,
    lease: &RequestAttachmentProcessingLease,
) -> anyhow::Result<ProcessingOutcome> {
    let work = process_claim(metadata, storage, pipeline, scratch, settings, lease).await;
    match work {
        Ok(completion) => {
            let source = completion.source.clone();
            match mark_source_validated(metadata, lease, source).await {
                Ok(true) => {}
                Ok(false) => {
                    delete_objects(storage, &completion.objects).await;
                    return Ok(ProcessingOutcome::LeaseLost);
                }
                Err(error) => {
                    delete_objects(storage, &completion.objects).await;
                    return record_source_validation_error(metadata, settings, lease, error).await;
                }
            }
            let now = crate::unix_now()?;
            let command = completion.command(lease, now);
            match metadata
                .media()
                .complete_processing_job(command)
                .await
                .map_err(db_error)?
            {
                MediaLeaseMutation::Applied(_) => Ok(ProcessingOutcome::Completed),
                MediaLeaseMutation::LeaseLost => {
                    delete_objects(storage, &completion.objects).await;
                    Ok(ProcessingOutcome::LeaseLost)
                }
            }
        }
        Err(WorkError::Failure {
            failure,
            validated_source,
        }) => {
            if let Some(source) = validated_source {
                match mark_source_validated(metadata, lease, source).await {
                    Ok(true) => {}
                    Ok(false) => return Ok(ProcessingOutcome::LeaseLost),
                    Err(error) => {
                        return record_source_validation_error(metadata, settings, lease, error)
                            .await;
                    }
                }
            }
            record_processing_failure(metadata, settings, lease, failure).await
        }
        Err(WorkError::LeaseLost) => Ok(ProcessingOutcome::LeaseLost),
        Err(WorkError::Database(error)) => Err(error),
    }
}

struct PreparedCompletion {
    source: ValidatedRequestAttachmentSource,
    derivatives: Vec<CompletedRequestAttachmentDerivative>,
    objects: Vec<MediaObject>,
}

impl PreparedCompletion {
    fn command(
        &self,
        lease: &RequestAttachmentProcessingLease,
        now_unix: u64,
    ) -> CompleteRequestAttachmentProcessingCommand {
        CompleteRequestAttachmentProcessingCommand {
            attachment_id: lease.attachment_id.clone(),
            lease_token: lease.lease_token.clone(),
            lease_generation: lease.lease_generation,
            source: self.source.clone(),
            derivatives: self.derivatives.clone(),
            now_unix,
        }
    }
}

enum WorkError {
    Failure {
        failure: RequestAttachmentFailure,
        validated_source: Option<ValidatedRequestAttachmentSource>,
    },
    LeaseLost,
    Database(anyhow::Error),
}

async fn process_claim(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    pipeline: &CodecPipeline,
    scratch: &ScratchSpace,
    settings: &WorkerSettings,
    lease: &RequestAttachmentProcessingLease,
) -> Result<PreparedCompletion, WorkError> {
    let manifest = metadata
        .media()
        .processing_source_manifest(lease, crate::unix_now().map_err(WorkError::Database)?)
        .await
        .map_err(|error| WorkError::Database(db_error(error)))?
        .ok_or(WorkError::LeaseLost)?;
    if manifest.size_bytes > settings.codec_limits.max_source_bytes {
        return Err(failure(
            RequestAttachmentFailureCode::MediaLimitExceeded,
            "media source exceeds the worker byte limit",
            false,
            None,
        ));
    }
    let source_object = media_object_from_manifest(&manifest).map_err(|error| {
        tracing::warn!(
            attachment_id = %lease.attachment_id,
            %error,
            "stored source manifest is invalid"
        );
        failure(
            RequestAttachmentFailureCode::CorruptMedia,
            "The uploaded file could not be read. Upload it again.",
            false,
            None,
        )
    })?;
    let job_dir = scratch.job().map_err(|error| {
        tracing::warn!(attachment_id = %lease.attachment_id, %error, "creating media scratch directory failed");
        failure(
            RequestAttachmentFailureCode::Internal,
            "Media processing failed. Retry processing.",
            true,
            None,
        )
    })?;
    let source_path = job_dir.path().join("source.bin");
    let mut source_file = tokio::fs::File::create(&source_path)
        .await
        .map_err(|error| {
            tracing::warn!(attachment_id = %lease.attachment_id, %error, "creating local media source failed");
            failure(
                RequestAttachmentFailureCode::Internal,
                "Media processing failed. Retry processing.",
                true,
                None,
            )
        })?;
    storage
        .download_to_writer(&source_object, &mut source_file)
        .await
        .map_err(|error| WorkError::Failure {
            failure: storage_failure(&error),
            validated_source: None,
        })?;
    source_file.sync_all().await.map_err(|error| {
        tracing::warn!(attachment_id = %lease.attachment_id, %error, "syncing local media source failed");
        failure(
            RequestAttachmentFailureCode::Internal,
            "Media processing failed. Retry processing.",
            true,
            None,
        )
    })?;
    drop(source_file);
    let (downloaded_bytes, downloaded_sha256) =
        sha256_file(&source_path).await.map_err(|error| {
            tracing::warn!(attachment_id = %lease.attachment_id, %error, "hashing local media source failed");
            failure(
                RequestAttachmentFailureCode::Internal,
                "Media processing failed. Retry processing.",
                true,
                None,
            )
        })?;
    if downloaded_bytes != manifest.size_bytes
        || !downloaded_sha256.eq_ignore_ascii_case(&manifest.sha256)
    {
        return Err(failure(
            RequestAttachmentFailureCode::CorruptMedia,
            "downloaded source does not match its immutable manifest",
            false,
            None,
        ));
    }

    let output = match pipeline.process(&source_path, job_dir.path()).await {
        Ok(output) => output,
        Err(error) => {
            tracing::warn!(
                attachment_id = %lease.attachment_id,
                kind = ?error.kind,
                error = %error,
                "media codec rejected or failed a source"
            );
            let validated = error
                .validated_source
                .as_ref()
                .map(|source| validated_source(source, &manifest));
            return Err(WorkError::Failure {
                failure: codec_failure(&error),
                validated_source: validated,
            });
        }
    };
    let validated_source = validated_source(&output.source, &manifest);

    let mut objects = Vec::new();
    let mut derivatives = Vec::new();
    for derivative in &output.derivatives {
        match upload_derivative(metadata, storage, lease, derivative).await {
            Ok((domain, object)) => {
                derivatives.push(domain);
                objects.push(object);
            }
            Err(error) => {
                delete_objects(storage, &objects).await;
                return match error {
                    UploadError::Storage(error) => Err(WorkError::Failure {
                        failure: storage_failure(&error),
                        validated_source: Some(validated_source),
                    }),
                    UploadError::Internal(error) => {
                        tracing::warn!(
                            attachment_id = %lease.attachment_id,
                            %error,
                            "local derivative upload preparation failed"
                        );
                        Err(WorkError::Failure {
                            failure: RequestAttachmentFailure {
                                code: RequestAttachmentFailureCode::Internal,
                                message: "Media processing failed. Retry processing.".to_owned(),
                                retryable: true,
                            },
                            validated_source: Some(validated_source),
                        })
                    }
                    UploadError::LeaseLost => Err(WorkError::LeaseLost),
                    UploadError::Database(error) => Err(WorkError::Database(error)),
                };
            }
        }
    }
    Ok(PreparedCompletion {
        source: validated_source,
        derivatives,
        objects,
    })
}

enum UploadError {
    Storage(MediaStorageError),
    Internal(anyhow::Error),
    LeaseLost,
    Database(anyhow::Error),
}

async fn upload_derivative(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    lease: &RequestAttachmentProcessingLease,
    derivative: &CodecDerivative,
) -> Result<(CompletedRequestAttachmentDerivative, MediaObject), UploadError> {
    let object_name = match derivative.kind {
        DerivativeKind::ImagePreview => "image-preview",
        DerivativeKind::VideoPlayback => "video-playback",
        DerivativeKind::VideoPoster => "video-poster",
    };
    let attempt = WriteAttempt::new(
        &lease.attachment_id,
        object_name,
        format!("{}-{}", lease.lease_token, lease.lease_generation),
    )
    .map_err(UploadError::Storage)?;
    let manifest_id = random_id("media").map_err(UploadError::Internal)?;
    let derivative_id = random_id("derivative").map_err(UploadError::Internal)?;
    let mut file = tokio::fs::File::open(&derivative.path)
        .await
        .map_err(io_storage_error)?;
    let mut staged = Vec::new();
    let mut whole_digest = Sha256::new();
    let mut total_bytes = 0_u64;
    loop {
        let mut bytes = Vec::with_capacity(MAX_CHUNK_BYTES);
        let read = (&mut file)
            .take(MAX_CHUNK_BYTES as u64)
            .read_to_end(&mut bytes)
            .await
            .map_err(io_storage_error)?;
        if read == 0 {
            break;
        }
        whole_digest.update(&bytes);
        total_bytes = total_bytes
            .checked_add(read as u64)
            .ok_or_else(|| io_storage_error(std::io::Error::other("derivative size overflow")))?;
        let part_number = u32::try_from(staged.len() + 1)
            .map_err(|_| io_storage_error(std::io::Error::other("too many derivative chunks")))?;
        let part = storage
            .plan_part(&attempt, part_number, &bytes)
            .map_err(UploadError::Storage)?;
        staged.push(part.clone());
        let reserved = metadata
            .media()
            .reserve_processing_object_key(
                &lease.attachment_id,
                &lease.lease_token,
                lease.lease_generation,
                &part.object_key,
                crate::unix_now().map_err(UploadError::Database)?,
            )
            .await
            .map_err(|error| UploadError::Database(db_error(error)))?;
        if matches!(reserved, MediaLeaseMutation::LeaseLost) {
            delete_parts(storage, &staged).await;
            return Err(UploadError::LeaseLost);
        }
        if let Err(error) = storage.write_part(&part, bytes).await {
            delete_parts(storage, &staged).await;
            return Err(UploadError::Storage(error));
        }
    }
    let digest = hex::encode(whole_digest.finalize());
    let object = match storage
        .seal_parts(derivative.media_type, total_bytes, &digest, staged.clone())
        .await
    {
        Ok(object) => object,
        Err(error) => {
            delete_parts(storage, &staged).await;
            return Err(UploadError::Storage(error));
        }
    };
    let stored_object = RequestAttachmentStoredObject {
        object_key: manifest_id.clone(),
        size_bytes: object.plaintext_bytes,
        sha256: object.sha256.clone(),
    };
    let domain = CompletedRequestAttachmentDerivative {
        derivative: RequestAttachmentDerivative {
            id: derivative_id,
            kind: domain_derivative_kind(&derivative.kind),
            media_type: derivative.media_type.to_owned(),
            object: stored_object,
            width: Some(derivative.width),
            height: Some(derivative.height),
            duration_millis: derivative.duration_millis,
        },
        manifest: completed_manifest(manifest_id, &object),
    };
    Ok((domain, object))
}

fn completed_manifest(id: String, object: &MediaObject) -> CompletedRequestMediaManifest {
    CompletedRequestMediaManifest {
        id,
        media_type: object.media_type.clone(),
        size_bytes: object.plaintext_bytes,
        sha256: object.sha256.clone(),
        chunks: object
            .chunks
            .iter()
            .map(|chunk| RequestMediaChunk {
                index: chunk.part_number,
                object_key: chunk.object_key.clone(),
                plaintext_offset: chunk.plaintext_offset,
                plaintext_size_bytes: chunk.plaintext_bytes,
                sha256: chunk.sha256.clone(),
            })
            .collect(),
    }
}

fn media_object_from_manifest(
    manifest: &RequestMediaManifest,
) -> Result<MediaObject, MediaStorageError> {
    MediaObject::new(
        &manifest.media_type,
        manifest.size_bytes,
        &manifest.sha256,
        manifest
            .chunks
            .iter()
            .map(|chunk| MediaChunk {
                part_number: chunk.index,
                plaintext_offset: chunk.plaintext_offset,
                plaintext_bytes: chunk.plaintext_size_bytes,
                sha256: chunk.sha256.clone(),
                object_key: chunk.object_key.clone(),
            })
            .collect(),
    )
}

fn validated_source(
    source: &ValidatedSource,
    manifest: &RequestMediaManifest,
) -> ValidatedRequestAttachmentSource {
    let (width, height, duration_millis) = match source.kind {
        MediaKind::Image => (Some(source.width), Some(source.height), None),
        MediaKind::Video => (
            Some(source.width),
            Some(source.height),
            source.duration_millis,
        ),
    };
    ValidatedRequestAttachmentSource {
        detected_media_type: source.media_type.to_owned(),
        size_bytes: manifest.size_bytes,
        sha256: manifest.sha256.clone(),
        width,
        height,
        duration_millis,
    }
}

async fn mark_source_validated(
    metadata: &MetadataStore,
    lease: &RequestAttachmentProcessingLease,
    source: ValidatedRequestAttachmentSource,
) -> anyhow::Result<bool> {
    let result = metadata
        .media()
        .mark_processing_source_validated(ValidateRequestAttachmentSourceCommand {
            attachment_id: lease.attachment_id.clone(),
            lease_token: lease.lease_token.clone(),
            lease_generation: lease.lease_generation,
            source,
            now_unix: crate::unix_now()?,
        })
        .await
        .map_err(anyhow::Error::new)?;
    Ok(matches!(result, MediaLeaseMutation::Applied(_)))
}

async fn record_source_validation_error(
    metadata: &MetadataStore,
    settings: &WorkerSettings,
    lease: &RequestAttachmentProcessingLease,
    error: anyhow::Error,
) -> anyhow::Result<ProcessingOutcome> {
    if error
        .downcast_ref::<scope_postgres::error::PostgresError>()
        .is_some_and(|error| error.kind == scope_postgres::error::PostgresErrorKind::InvalidInput)
    {
        return record_processing_failure(metadata, settings, lease, RequestAttachmentFailure {
            code: RequestAttachmentFailureCode::CorruptMedia,
            message: "The uploaded media does not match its declared type. Upload it with the correct file type.".to_owned(),
            retryable: false,
        }).await;
    }
    Err(error)
}

async fn record_processing_failure(
    metadata: &MetadataStore,
    settings: &WorkerSettings,
    lease: &RequestAttachmentProcessingLease,
    failure: RequestAttachmentFailure,
) -> anyhow::Result<ProcessingOutcome> {
    let now = crate::unix_now()?;
    let retry_at_unix = (failure.retryable && lease.attempt < settings.max_attempts)
        .then(|| now.saturating_add(retry_delay(lease.attempt).as_secs()));
    let result = metadata
        .media()
        .fail_processing_job(FailRequestAttachmentProcessingCommand {
            attachment_id: lease.attachment_id.clone(),
            lease_token: lease.lease_token.clone(),
            lease_generation: lease.lease_generation,
            failure,
            now_unix: now,
            retry_at_unix,
        })
        .await
        .map_err(db_error)?;
    Ok(match result {
        MediaLeaseMutation::Applied(_) => ProcessingOutcome::Failed,
        MediaLeaseMutation::LeaseLost => ProcessingOutcome::LeaseLost,
    })
}

#[derive(Debug)]
enum HeartbeatError {
    LeaseLost,
    Database(anyhow::Error),
}

async fn supervise_processing_claim<T, F, R, RFut>(
    future: F,
    lease_duration: Duration,
    mut renew: R,
) -> Result<T, HeartbeatError>
where
    F: Future<Output = T>,
    R: FnMut() -> RFut,
    RFut: Future<Output = anyhow::Result<bool>>,
{
    tokio::pin!(future);
    let mut heartbeat = tokio::time::interval(heartbeat_interval(lease_duration));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    heartbeat.tick().await;
    loop {
        tokio::select! {
            result = &mut future => return Ok(result),
            _ = heartbeat.tick() => {
                let renewal = renew();
                tokio::pin!(renewal);
                let renewed = tokio::select! {
                    result = &mut future => return Ok(result),
                    renewed = &mut renewal => renewed,
                };
                match renewed {
                    Ok(true) => {}
                    Ok(false) => return Err(HeartbeatError::LeaseLost),
                    Err(error) => return Err(HeartbeatError::Database(error)),
                }
            }
        }
    }
}

async fn renew_processing(
    metadata: &MetadataStore,
    lease: &RequestAttachmentProcessingLease,
    duration: Duration,
    now_unix: u64,
) -> anyhow::Result<bool> {
    metadata
        .media()
        .renew_processing_lease(
            &lease.attachment_id,
            &lease.lease_token,
            lease.lease_generation,
            now_unix,
            lease_expiry(now_unix, duration)?,
        )
        .await
        .map_err(db_error)
}

async fn dependencies_ready(
    metadata: &MetadataStore,
    storage: &MediaStorage,
    health: &WorkerHealth,
) -> bool {
    let schema = metadata.admin().readiness_check().await;
    let object_store = storage.readiness_check().await;
    if schema.is_ok() && object_store.is_ok() {
        health.mark_dependencies_ready();
        return true;
    }
    health.mark_dependencies_waiting();
    if let Err(error) = schema {
        tracing::warn!(error = %error.message, "media worker schema fence is unavailable");
    }
    if let Err(error) = object_store {
        tracing::warn!(%error, "media worker object storage is unavailable");
    }
    false
}

async fn sha256_file(path: &Path) -> anyhow::Result<(u64, String)> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut digest = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| anyhow::anyhow!("media source size overflow"))?;
    }
    Ok((total, hex::encode(digest.finalize())))
}

fn codec_failure(error: &CodecFailure) -> RequestAttachmentFailure {
    let code = match error.kind {
        CodecFailureKind::InvalidMedia => RequestAttachmentFailureCode::InvalidMedia,
        CodecFailureKind::CorruptMedia => RequestAttachmentFailureCode::CorruptMedia,
        CodecFailureKind::UnsupportedMedia => RequestAttachmentFailureCode::UnsupportedMedia,
        CodecFailureKind::MediaLimitExceeded => RequestAttachmentFailureCode::MediaLimitExceeded,
        CodecFailureKind::CodecFailed => RequestAttachmentFailureCode::CodecFailed,
        CodecFailureKind::Internal => RequestAttachmentFailureCode::Internal,
    };
    RequestAttachmentFailure {
        code,
        message: codec_user_message(error.kind).to_owned(),
        retryable: error.retryable(),
    }
}

fn storage_failure(error: &MediaStorageError) -> RequestAttachmentFailure {
    let (code, retryable) = match error.kind {
        MediaStorageErrorKind::Integrity | MediaStorageErrorKind::NotFound => {
            (RequestAttachmentFailureCode::CorruptMedia, false)
        }
        MediaStorageErrorKind::InvalidInput => (RequestAttachmentFailureCode::Internal, false),
        MediaStorageErrorKind::CapacityExhausted
        | MediaStorageErrorKind::Internal
        | MediaStorageErrorKind::ServiceUnavailable => {
            (RequestAttachmentFailureCode::StorageUnavailable, true)
        }
    };
    RequestAttachmentFailure {
        code,
        message: match code {
            RequestAttachmentFailureCode::CorruptMedia => {
                "The uploaded file could not be read. Upload it again."
            }
            RequestAttachmentFailureCode::StorageUnavailable => {
                "Media storage is temporarily unavailable. Retry processing."
            }
            _ => "Media processing failed. Retry processing.",
        }
        .to_owned(),
        retryable,
    }
}

fn failure(
    code: RequestAttachmentFailureCode,
    message: impl AsRef<str>,
    retryable: bool,
    validated_source: Option<ValidatedRequestAttachmentSource>,
) -> WorkError {
    WorkError::Failure {
        failure: RequestAttachmentFailure {
            code,
            message: message.as_ref().to_owned(),
            retryable,
        },
        validated_source,
    }
}

fn domain_derivative_kind(kind: &DerivativeKind) -> RequestAttachmentDerivativeKind {
    match kind {
        DerivativeKind::ImagePreview => RequestAttachmentDerivativeKind::ImagePreview,
        DerivativeKind::VideoPlayback => RequestAttachmentDerivativeKind::VideoPlayback,
        DerivativeKind::VideoPoster => RequestAttachmentDerivativeKind::VideoPoster,
    }
}

fn random_id(prefix: &str) -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("secure random generation failed: {error}"))?;
    Ok(format!("{prefix}-{}", hex::encode(bytes)))
}

fn lease_expiry(now_unix: u64, duration: Duration) -> anyhow::Result<u64> {
    now_unix
        .checked_add(duration.as_secs())
        .ok_or_else(|| anyhow::anyhow!("media lease expiry overflow"))
}

fn heartbeat_interval(lease_duration: Duration) -> Duration {
    (lease_duration / 3).max(Duration::from_millis(10))
}

fn retry_delay(attempt: u32) -> Duration {
    Duration::from_secs(5_u64.saturating_mul(1_u64 << attempt.min(7)).min(600))
}

fn io_storage_error(error: std::io::Error) -> UploadError {
    UploadError::Internal(anyhow::Error::new(error).context("reading derivative"))
}

fn db_error(error: scope_postgres::error::PostgresError) -> anyhow::Error {
    anyhow::anyhow!(error.message)
}

fn codec_user_message(kind: CodecFailureKind) -> &'static str {
    match kind {
        CodecFailureKind::InvalidMedia => "The uploaded file is not valid media.",
        CodecFailureKind::CorruptMedia => {
            "The uploaded file is corrupt or could not be fully decoded."
        }
        CodecFailureKind::UnsupportedMedia => "This media format or codec is not supported.",
        CodecFailureKind::MediaLimitExceeded => "The uploaded file exceeds the media limits.",
        CodecFailureKind::CodecFailed => "Media conversion failed. Retry processing.",
        CodecFailureKind::Internal => "Media processing failed. Retry processing.",
    }
}

async fn delete_parts(storage: &MediaStorage, parts: &[StagedMediaPart]) {
    for part in parts {
        if let Err(error) = storage.delete_staged_part(part).await {
            tracing::warn!(%error, object_key = %part.object_key, "failed to discard staged derivative part");
        }
    }
}

async fn delete_objects(storage: &MediaStorage, objects: &[MediaObject]) {
    for object in objects {
        if let Err(error) = storage.delete_object(object).await {
            tracing::warn!(%error, "failed to discard unpublished media derivative");
        }
    }
}

async fn wait_or_shutdown(duration: Duration) -> bool {
    tokio::select! {
        _ = crate::shutdown_signal() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn poll_waits_when_idle_or_after_an_error() {
        assert!(should_wait_after_poll(&Ok(ProcessingOutcome::NoJob)));
        assert!(should_wait_after_poll(&Err(anyhow::anyhow!(
            "database unavailable"
        ))));
        assert!(!should_wait_after_poll(&Ok(ProcessingOutcome::Completed)));
        assert!(!should_wait_after_poll(&Ok(ProcessingOutcome::Failed)));
        assert!(!should_wait_after_poll(&Ok(ProcessingOutcome::LeaseLost)));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn heartbeat_keeps_a_claim_alive_beyond_its_initial_lease() {
        let lease_duration = Duration::from_millis(180);
        let expires_at = Arc::new(Mutex::new(tokio::time::Instant::now() + lease_duration));
        let renewals = Arc::new(AtomicUsize::new(0));
        let renewal_expiry = Arc::clone(&expires_at);
        let renewal_count = Arc::clone(&renewals);

        let result = supervise_processing_claim(
            async {
                tokio::time::sleep(Duration::from_millis(650)).await;
                42
            },
            lease_duration,
            move || {
                let expires_at = Arc::clone(&renewal_expiry);
                let renewals = Arc::clone(&renewal_count);
                async move {
                    let now = tokio::time::Instant::now();
                    let mut expires_at = expires_at.lock().expect("lease expiry lock");
                    if now >= *expires_at {
                        return Ok(false);
                    }
                    *expires_at = now + lease_duration;
                    renewals.fetch_add(1, Ordering::Relaxed);
                    Ok(true)
                }
            },
        )
        .await
        .expect("claim should remain leased");

        assert_eq!(result, 42);
        assert!(renewals.load(Ordering::Relaxed) >= 6);
        assert!(*expires_at.lock().expect("lease expiry lock") > tokio::time::Instant::now());
    }
}
