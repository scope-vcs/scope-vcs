mod failures;

use crate::lease::{HeartbeatError, supervise_lease};
use crate::{
    codec::{CodecDerivative, CodecPipeline, ValidatedSource},
    config::WorkerSettings,
    health::WorkerHealth,
    runtime::{JobLoop, JobOutcome, lease_expiry, retry_delay},
    scratch::ScratchSpace,
};
use failures::{DECLARED_TYPE_MISMATCH, codec_failure, storage_failure};
use scope_domain::requests::attachments::{
    RequestAttachmentDerivative, RequestAttachmentDerivativeKind, RequestAttachmentFailure,
    RequestAttachmentFailureCode, RequestAttachmentImageMetadata, RequestAttachmentKind,
    RequestAttachmentProcessingLane, RequestAttachmentProcessingLease, RequestAttachmentSource,
    RequestAttachmentStagedPart, RequestAttachmentStoredObject, RequestAttachmentVideoMetadata,
    RequestMediaObject, processing_attempts_exhausted,
};
use scope_media_storage::{
    MAX_CHUNK_BYTES, MediaStorage, MediaStorageError, WriteAttempt, seal_parts,
};
use scope_postgres::db::{
    CompleteRequestAttachmentProcessingCommand, CompletedRequestAttachmentDerivative,
    FailRequestAttachmentProcessingCommand, MediaLeaseMutation, MetadataStore,
    ValidateRequestAttachmentSourceCommand,
};
use scope_service_runtime::{random_hex, unix_now};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tracing::Instrument as _;

pub async fn run_processing_loop(
    lane: RequestAttachmentProcessingLane,
    metadata: MetadataStore,
    storage: MediaStorage,
    pipeline: CodecPipeline,
    scratch: ScratchSpace,
    settings: WorkerSettings,
    health: WorkerHealth,
) -> anyhow::Result<()> {
    JobLoop {
        worker_loop: lane.into(),
        metadata: &metadata,
        storage: &storage,
        health: &health,
        poll_interval: settings.poll_interval,
    }
    .run(|| {
        process_next_job(
            lane, &metadata, &storage, &pipeline, &scratch, &settings, &health,
        )
    })
    .await
}

async fn process_next_job(
    lane: RequestAttachmentProcessingLane,
    metadata: &MetadataStore,
    storage: &MediaStorage,
    pipeline: &CodecPipeline,
    scratch: &ScratchSpace,
    settings: &WorkerSettings,
    health: &WorkerHealth,
) -> anyhow::Result<JobOutcome> {
    let now = unix_now()?;
    let token = random_hex("lease-", 16)?;
    let Some((lease, queued_trace)) = metadata
        .media()
        .claim_processing_job(
            lane,
            &token,
            now,
            lease_expiry(now, settings.lease_duration)?,
        )
        .await?
    else {
        return Ok(JobOutcome::NoJob);
    };
    tracing::info!(
        attachment_id = %lease.attachment_id,
        kind = ?lease.kind,
        attempt = lease.attempt,
        lease_generation = lease.lease_generation,
        "claimed media processing job"
    );
    let span = tracing::info_span!(
        parent: None,
        "job.media_processing",
        otel.kind = "consumer",
        scope.job.kind = "media_processing",
        scope.job.id = %lease.attachment_id,
        scope.job.attempt = lease.attempt,
    );
    queued_trace.link(&span);
    let _activity = health.activity(lane.into());
    let work = settle_claim(metadata, storage, pipeline, scratch, settings, &lease);
    match supervise_lease(work, settings.lease_duration, || async {
        let now = unix_now()?;
        Ok(metadata
            .media()
            .renew_processing_lease(
                &lease.attachment_id,
                &lease.lease_token,
                lease.lease_generation,
                now,
                lease_expiry(now, settings.lease_duration)?,
            )
            .await?)
    })
    .instrument(span)
    .await
    {
        Ok(outcome) => outcome,
        Err(HeartbeatError::LeaseLost) => Ok(JobOutcome::LeaseLost),
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
) -> anyhow::Result<JobOutcome> {
    let work = process_claim(metadata, storage, pipeline, scratch, settings, lease).await;
    match work {
        Ok(completion) => {
            match mark_source_validated(metadata, lease, completion.source.clone()).await {
                Ok(true) => {}
                Ok(false) => {
                    delete_objects(storage, &completion.objects).await;
                    return Ok(JobOutcome::LeaseLost);
                }
                Err(error) => {
                    delete_objects(storage, &completion.objects).await;
                    return record_source_validation_error(metadata, lease, error).await;
                }
            }
            let command = CompleteRequestAttachmentProcessingCommand {
                attachment_id: lease.attachment_id.clone(),
                lease_token: lease.lease_token.clone(),
                lease_generation: lease.lease_generation,
                source: completion.source,
                derivatives: completion.derivatives,
                now_unix: unix_now()?,
            };
            match metadata.media().complete_processing_job(command).await? {
                MediaLeaseMutation::Applied(_) => Ok(JobOutcome::Completed),
                MediaLeaseMutation::LeaseLost => {
                    delete_objects(storage, &completion.objects).await;
                    Ok(JobOutcome::LeaseLost)
                }
            }
        }
        Err(WorkError::Failure {
            failure,
            validated_source,
        }) => {
            if let Some(source) = validated_source {
                match mark_source_validated(metadata, lease, *source).await {
                    Ok(true) => {}
                    Ok(false) => return Ok(JobOutcome::LeaseLost),
                    Err(error) => {
                        return record_source_validation_error(metadata, lease, error).await;
                    }
                }
            }
            record_processing_failure(metadata, lease, failure).await
        }
        Err(WorkError::LeaseLost) => Ok(JobOutcome::LeaseLost),
        Err(WorkError::Database(error)) => Err(error),
    }
}

struct PreparedCompletion {
    source: RequestAttachmentSource,
    derivatives: Vec<CompletedRequestAttachmentDerivative>,
    objects: Vec<RequestMediaObject>,
}

enum WorkError {
    Failure {
        failure: RequestAttachmentFailure,
        validated_source: Option<Box<RequestAttachmentSource>>,
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
        .processing_source_manifest(
            lease,
            unix_now().map_err(|error| WorkError::Database(error.into()))?,
        )
        .await
        .map_err(|error| WorkError::Database(error.into()))?
        .ok_or(WorkError::LeaseLost)?;
    if manifest.plaintext_bytes > settings.codec_limits.max_source_bytes {
        return Err(failure(
            RequestAttachmentFailureCode::MediaLimitExceeded,
            "media source exceeds the worker byte limit",
            false,
        ));
    }
    let job_dir = scratch
        .job()
        .map_err(local_failure(lease, "creating media scratch directory"))?;
    let source_path = job_dir.path().join("source.bin");
    let mut source_file = tokio::fs::File::create(&source_path)
        .await
        .map_err(local_failure(lease, "creating local media source"))?;
    storage
        .download_to_writer(&manifest, &mut source_file)
        .await
        .map_err(|error| WorkError::Failure {
            failure: storage_failure(&error),
            validated_source: None,
        })?;
    drop(source_file);

    let output = match pipeline
        .process(&source_path, job_dir.path(), lease.kind)
        .await
    {
        Ok(output) => {
            tracing::info!(
                attachment_id = %lease.attachment_id,
                playback = ?output.playback,
                "media codec produced derivatives"
            );
            output
        }
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
                .map(|source| Box::new(validated_source(source, &manifest)));
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
                        validated_source: Some(Box::new(validated_source)),
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
                            validated_source: Some(Box::new(validated_source)),
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
) -> Result<(CompletedRequestAttachmentDerivative, RequestMediaObject), UploadError> {
    let object_name = match derivative.kind {
        RequestAttachmentDerivativeKind::ImagePreview => "image-preview",
        RequestAttachmentDerivativeKind::VideoPlayback => "video-playback",
        RequestAttachmentDerivativeKind::VideoPoster => "video-poster",
    };
    let attempt = WriteAttempt::new(
        &lease.attachment_id,
        object_name,
        format!("{}-{}", lease.lease_token, lease.lease_generation),
    )
    .map_err(UploadError::Storage)?;
    let manifest_id =
        random_hex("media-", 16).map_err(|error| UploadError::Internal(error.into()))?;
    let derivative_id =
        random_hex("derivative-", 16).map_err(|error| UploadError::Internal(error.into()))?;
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
                unix_now().map_err(|error| UploadError::Database(error.into()))?,
            )
            .await
            .map_err(|error| UploadError::Database(error.into()))?;
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
    let object = match seal_parts(derivative.media_type, total_bytes, &digest, staged.clone()) {
        Ok(object) => object,
        Err(error) => {
            delete_parts(storage, &staged).await;
            return Err(UploadError::Storage(error));
        }
    };
    let stored_object = RequestAttachmentStoredObject {
        object_key: manifest_id,
        size_bytes: object.plaintext_bytes,
        sha256: object.sha256.clone(),
    };
    let domain = CompletedRequestAttachmentDerivative {
        derivative: RequestAttachmentDerivative {
            id: derivative_id,
            kind: derivative.kind,
            media_type: derivative.media_type.to_owned(),
            object: stored_object,
            width: Some(derivative.width),
            height: Some(derivative.height),
            duration_millis: derivative.duration_millis,
        },
        chunks: object.chunks.clone(),
    };
    Ok((domain, object))
}

fn validated_source(
    source: &ValidatedSource,
    manifest: &RequestMediaObject,
) -> RequestAttachmentSource {
    let (image, video) = match source.kind {
        RequestAttachmentKind::Photo => (
            Some(RequestAttachmentImageMetadata {
                width: source.width,
                height: source.height,
            }),
            None,
        ),
        RequestAttachmentKind::Video => (
            None,
            source
                .duration_millis
                .map(|duration_millis| RequestAttachmentVideoMetadata {
                    width: source.width,
                    height: source.height,
                    duration_millis,
                }),
        ),
    };
    RequestAttachmentSource {
        detected_media_type: source.media_type.to_owned(),
        size_bytes: manifest.plaintext_bytes,
        sha256: manifest.sha256.clone(),
        image,
        video,
    }
}

async fn mark_source_validated(
    metadata: &MetadataStore,
    lease: &RequestAttachmentProcessingLease,
    source: RequestAttachmentSource,
) -> anyhow::Result<bool> {
    let result = metadata
        .media()
        .mark_processing_source_validated(ValidateRequestAttachmentSourceCommand {
            attachment_id: lease.attachment_id.clone(),
            lease_token: lease.lease_token.clone(),
            lease_generation: lease.lease_generation,
            source,
            now_unix: unix_now()?,
        })
        .await?;
    Ok(matches!(result, MediaLeaseMutation::Applied(_)))
}

async fn record_source_validation_error(
    metadata: &MetadataStore,
    lease: &RequestAttachmentProcessingLease,
    error: anyhow::Error,
) -> anyhow::Result<JobOutcome> {
    if error
        .downcast_ref::<scope_postgres::error::PostgresError>()
        .is_some_and(|error| error.kind == scope_postgres::error::PostgresErrorKind::InvalidInput)
    {
        return record_processing_failure(
            metadata,
            lease,
            RequestAttachmentFailure {
                code: RequestAttachmentFailureCode::CorruptMedia,
                message: DECLARED_TYPE_MISMATCH.to_owned(),
                retryable: false,
            },
        )
        .await;
    }
    Err(error)
}

async fn record_processing_failure(
    metadata: &MetadataStore,
    lease: &RequestAttachmentProcessingLease,
    failure: RequestAttachmentFailure,
) -> anyhow::Result<JobOutcome> {
    let now = unix_now()?;
    let retry_at_unix = (failure.retryable && !processing_attempts_exhausted(lease.attempt))
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
        .await?;
    Ok(match result {
        MediaLeaseMutation::Applied(_) => JobOutcome::Failed,
        MediaLeaseMutation::LeaseLost => JobOutcome::LeaseLost,
    })
}

fn failure(code: RequestAttachmentFailureCode, message: &str, retryable: bool) -> WorkError {
    WorkError::Failure {
        failure: RequestAttachmentFailure {
            code,
            message: message.to_owned(),
            retryable,
        },
        validated_source: None,
    }
}

fn local_failure<E: std::fmt::Display>(
    lease: &RequestAttachmentProcessingLease,
    context: &'static str,
) -> impl FnOnce(E) -> WorkError {
    let attachment_id = lease.attachment_id.clone();
    move |error| {
        tracing::warn!(%attachment_id, %error, "{context} failed");
        failure(
            RequestAttachmentFailureCode::Internal,
            "Media processing failed. Retry processing.",
            true,
        )
    }
}

fn io_storage_error(error: std::io::Error) -> UploadError {
    UploadError::Internal(anyhow::Error::new(error).context("reading derivative"))
}

async fn delete_parts(storage: &MediaStorage, parts: &[RequestAttachmentStagedPart]) {
    for part in parts {
        if let Err(error) = storage.delete_object_key(&part.object_key).await {
            tracing::warn!(%error, object_key = %part.object_key, "failed to discard staged derivative part");
        }
    }
}

async fn delete_objects(storage: &MediaStorage, objects: &[RequestMediaObject]) {
    for object in objects {
        if let Err(error) = storage.delete_object(object).await {
            tracing::warn!(%error, "failed to discard unpublished media derivative");
        }
    }
}
