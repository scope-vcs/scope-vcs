use crate::{
    AppState,
    error::ServiceError,
    ranges::{RequestedRange, requested_range},
};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::{Path, Query, State},
    http::{
        HeaderMap, HeaderValue, Method, StatusCode,
        header::{
            ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE,
            CONTENT_TYPE, ETAG,
        },
    },
    response::Response,
};
use bytes::Bytes;
use scope_api_contract::{RequestAttachmentMediaTarget, RequestAttachmentPartReceiptResponse};
use scope_domain::requests::attachments::RequestAttachmentPartReceipt;
use scope_media_storage::{
    MAX_CHUNK_BYTES, MediaByteStream, MediaChunk, MediaObject, StagedMediaPart, WriteAttempt,
};
use scope_postgres::db::{
    RequestMediaManifest, RequestMediaObjectTarget, ReserveUploadPartResult, StorePartResult,
    StoredRequestAttachmentPart,
};
use serde::Deserialize;
use std::{
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::OwnedSemaphorePermit;
use tokio_stream::Stream;

#[derive(Deserialize)]
pub(crate) struct GrantQuery {
    grant: String,
}

pub(crate) async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

pub(crate) async fn readyz(State(state): State<AppState>) -> Result<StatusCode, ServiceError> {
    state.metadata.admin().readiness_check().await?;
    state.storage.readiness_check().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn put_upload_part(
    State(state): State<AppState>,
    Path((upload_id, part_number)): Path<(String, u32)>,
    request: axum::extract::Request,
) -> Result<Json<RequestAttachmentPartReceiptResponse>, ServiceError> {
    let now_unix = unix_now()?;
    let claims = state
        .verifier
        .verify_upload(request.headers(), &upload_id, now_unix)?;
    let authorized = state
        .metadata
        .media()
        .request_attachment_for_viewer(
            &claims.request_id,
            &claims.attachment_id,
            Some(&claims.uploader_user_id),
        )
        .await?
        .filter(|authorized| {
            authorized.repository_id == claims.repository_id
                && authorized.attachment.upload_id == claims.upload_id
                && authorized.attachment.uploader_user_id == claims.uploader_user_id
        })
        .ok_or_else(ServiceError::not_found)?;
    drop(authorized);

    // Acquire before reading the body so queued uploads cannot each retain an 8 MiB part.
    let (_permit, bytes) = buffer_upload_body(
        state.upload_slots.clone(),
        request.into_body(),
        Duration::from_secs(60),
    )
    .await?;
    let part_bytes = bytes.to_vec();
    drop(bytes);
    let attempt = WriteAttempt::new(&claims.attachment_id, "original", &claims.upload_id)?;
    let planned = state
        .storage
        .plan_part(&attempt, part_number, &part_bytes)?;
    let proposed = stored_part(&planned);
    let write_token = random_token("mpw_")?;
    let write_started_at_unix = unix_now()?;
    let write_expires_at_unix = write_started_at_unix
        .checked_add(60)
        .ok_or_else(|| ServiceError::internal("media part write expiry overflowed"))?;
    let reserved = match state
        .metadata
        .media()
        .reserve_upload_part(
            &claims.attachment_id,
            &claims.upload_id,
            &claims.uploader_user_id,
            proposed,
            &write_token,
            write_started_at_unix,
            write_expires_at_unix,
        )
        .await?
    {
        ReserveUploadPartResult::Write(part) => part,
        ReserveUploadPartResult::Stored(part) => return Ok(Json(part.receipt.into())),
        ReserveUploadPartResult::Busy => {
            return Err(ServiceError::unavailable(
                "media part is already being stored; retry",
            ));
        }
    };
    let reserved_part = StagedMediaPart {
        part_number: reserved.receipt.part_number,
        size_bytes: reserved.receipt.size_bytes,
        sha256: reserved.receipt.sha256.clone(),
        object_key: reserved.object_key.clone(),
    };
    state.storage.write_part(&reserved_part, part_bytes).await?;
    let store_result = state
        .metadata
        .media()
        .mark_upload_part_stored(
            &claims.attachment_id,
            &claims.upload_id,
            &claims.uploader_user_id,
            part_number,
            &reserved.object_key,
            &write_token,
            unix_now()?,
        )
        .await;
    match store_result {
        Ok(StorePartResult::Recorded | StorePartResult::AlreadyRecorded(_)) => {}
        Ok(StorePartResult::WriteLeaseLost) => {
            state
                .storage
                .delete_object_key(&reserved.object_key)
                .await?;
            return Err(ServiceError::conflict(
                "media part write lease was lost; retry the part",
            ));
        }
        // The reservation remains durable on an ambiguous database error. Deleting here could
        // remove an object whose Stored transition committed before the connection failed.
        Err(error) => return Err(error.into()),
    }
    Ok(Json(reserved.receipt.into()))
}

fn stored_part(part: &StagedMediaPart) -> StoredRequestAttachmentPart {
    StoredRequestAttachmentPart {
        receipt: RequestAttachmentPartReceipt {
            part_number: part.part_number,
            size_bytes: part.size_bytes,
            sha256: part.sha256.clone(),
        },
        object_key: part.object_key.clone(),
    }
}

async fn buffer_upload_body(
    slots: Arc<tokio::sync::Semaphore>,
    body: Body,
    timeout: Duration,
) -> Result<(OwnedSemaphorePermit, Bytes), ServiceError> {
    let permit = slots
        .try_acquire_owned()
        .map_err(|_| ServiceError::unavailable("media upload capacity is busy; retry the part"))?;
    let bytes = tokio::time::timeout(timeout, to_bytes(body, MAX_CHUNK_BYTES))
        .await
        .map_err(|_| ServiceError::request_timeout("media part body timed out"))?
        .map_err(|_| ServiceError::payload_too_large("media part exceeds the 8 MiB limit"))?;
    Ok((permit, bytes))
}

pub(crate) async fn get_original(
    state: State<AppState>,
    path: Path<String>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
) -> Result<Response, ServiceError> {
    serve_media(state.0, path.0, None, query.0.grant, headers, Method::GET).await
}

pub(crate) async fn head_original(
    state: State<AppState>,
    path: Path<String>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
) -> Result<Response, ServiceError> {
    serve_media(state.0, path.0, None, query.0.grant, headers, Method::HEAD).await
}

pub(crate) async fn get_derivative(
    state: State<AppState>,
    Path((attachment_id, derivative_id)): Path<(String, String)>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
) -> Result<Response, ServiceError> {
    serve_media(
        state.0,
        attachment_id,
        Some(derivative_id),
        query.0.grant,
        headers,
        Method::GET,
    )
    .await
}

pub(crate) async fn head_derivative(
    state: State<AppState>,
    Path((attachment_id, derivative_id)): Path<(String, String)>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
) -> Result<Response, ServiceError> {
    serve_media(
        state.0,
        attachment_id,
        Some(derivative_id),
        query.0.grant,
        headers,
        Method::HEAD,
    )
    .await
}

async fn serve_media(
    state: AppState,
    attachment_id: String,
    derivative_id: Option<String>,
    grant: String,
    headers: HeaderMap,
    method: Method,
) -> Result<Response, ServiceError> {
    let target = match &derivative_id {
        Some(derivative_id) => RequestAttachmentMediaTarget::Derivative {
            derivative_id: derivative_id.clone(),
        },
        None => RequestAttachmentMediaTarget::Original,
    };
    let claims = state
        .verifier
        .verify_media(&grant, &attachment_id, &target, unix_now()?)?;
    let authorized = state
        .metadata
        .media()
        .request_attachment_for_viewer(
            &claims.request_id,
            &attachment_id,
            claims.viewer_user_id.as_deref(),
        )
        .await?
        .filter(|authorized| authorized.repository_id == claims.repository_id)
        .ok_or_else(ServiceError::not_found)?;
    let original_filename = derivative_id
        .is_none()
        .then(|| authorized.attachment.filename.clone());
    let db_target = match derivative_id.as_deref() {
        Some(id) => RequestMediaObjectTarget::Derivative(id),
        None => RequestMediaObjectTarget::Original,
    };
    // This second lookup is intentional: it is the live, target-specific policy check for
    // this request and cannot be replaced by the signed grant's earlier authorization.
    let manifest = state
        .metadata
        .media()
        .authorized_media_manifest(
            &claims.request_id,
            &attachment_id,
            claims.viewer_user_id.as_deref(),
            db_target,
        )
        .await?
        .ok_or_else(ServiceError::not_found)?;
    let object = media_object(&manifest)?;
    let range = requested_range(&headers, object.plaintext_bytes)?;
    let permit = if method == Method::GET && object.plaintext_bytes > 0 {
        Some(
            state
                .read_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| ServiceError::unavailable("media read capacity is busy; retry"))?,
        )
    } else {
        None
    };
    media_response(
        &state.storage,
        &object,
        range,
        method,
        permit,
        original_filename.as_deref(),
    )
    .await
}

fn media_object(manifest: &RequestMediaManifest) -> Result<MediaObject, ServiceError> {
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
    .map_err(Into::into)
}

async fn media_response(
    storage: &scope_media_storage::MediaStorage,
    object: &MediaObject,
    requested: RequestedRange,
    method: Method,
    permit: Option<OwnedSemaphorePermit>,
    download_filename: Option<&str>,
) -> Result<Response, ServiceError> {
    let (status, start, end) = match requested {
        RequestedRange::Full if object.plaintext_bytes == 0 => (StatusCode::OK, 0, None),
        RequestedRange::Full => (StatusCode::OK, 0, Some(object.plaintext_bytes - 1)),
        RequestedRange::Partial(range) => (
            StatusCode::PARTIAL_CONTENT,
            *range.start(),
            Some(*range.end()),
        ),
    };
    let length = end.map_or(0, |end| end - start + 1);
    let body = if method == Method::HEAD || length == 0 {
        Body::empty()
    } else {
        let stream = storage
            .read_range(object, start..=end.expect("nonempty media range"))
            .await?;
        Body::from_stream(PermittedStream::new(
            stream,
            permit.expect("GET read permit"),
        ))
    };
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let response_headers = response.headers_mut();
    response_headers.insert(ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response_headers.insert(CACHE_CONTROL, HeaderValue::from_static("private, no-store"));
    response_headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string())
            .map_err(|_| ServiceError::internal("invalid media content length"))?,
    );
    response_headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_str(&object.media_type)
            .map_err(|_| ServiceError::internal("invalid media content type"))?,
    );
    response_headers.insert(
        ETAG,
        HeaderValue::from_str(&format!("\"{}\"", object.sha256))
            .map_err(|_| ServiceError::internal("invalid media etag"))?,
    );
    if status == StatusCode::PARTIAL_CONTENT {
        response_headers.insert(
            CONTENT_RANGE,
            HeaderValue::from_str(&format!(
                "bytes {start}-{}/{}",
                end.expect("partial range end"),
                object.plaintext_bytes
            ))
            .map_err(|_| ServiceError::internal("invalid media content range"))?,
        );
    }
    if let Some(filename) = download_filename {
        response_headers.insert(
            CONTENT_DISPOSITION,
            HeaderValue::from_str(&content_disposition(filename))
                .map_err(|_| ServiceError::internal("invalid media content disposition"))?,
        );
    }
    Ok(response)
}

fn content_disposition(filename: &str) -> String {
    let fallback = filename
        .chars()
        .take(150)
        .map(|character| match character {
            ' '..='!' | '#'..='[' | ']'..='~' if character.is_ascii() => character,
            _ => '_',
        })
        .collect::<String>();
    let fallback = if fallback.trim().is_empty() {
        "download"
    } else {
        &fallback
    };
    let encoded = filename
        .as_bytes()
        .iter()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'&'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
            {
                vec![char::from(*byte)]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect::<String>();
    format!("attachment; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

struct PermittedStream {
    inner: MediaByteStream,
    _permit: OwnedSemaphorePermit,
}

impl PermittedStream {
    fn new(inner: MediaByteStream, permit: OwnedSemaphorePermit) -> Self {
        Self {
            inner,
            _permit: permit,
        }
    }
}

impl Stream for PermittedStream {
    type Item = Result<Bytes, scope_media_storage::MediaStorageError>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(context)
    }
}

fn unix_now() -> Result<u64, ServiceError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| ServiceError::internal(error.to_string()))
}

fn random_token(prefix: &str) -> Result<String, ServiceError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        ServiceError::internal(format!("media token generation failed: {error}"))
    })?;
    Ok(format!("{prefix}{}", hex::encode(random)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_media_storage::WriteAttempt;
    use scope_object_store::MemoryObjectStore;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use tokio::sync::Semaphore;
    use tokio_stream::wrappers::ReceiverStream;

    async fn fixture() -> (scope_media_storage::MediaStorage, MediaObject) {
        let storage = scope_media_storage::MediaStorage::encrypted(
            Arc::new(MemoryObjectStore::new()),
            [21; 32],
            1,
        )
        .unwrap();
        let attempt = WriteAttempt::new("att_test", "original", "upload_test").unwrap();
        let plaintext = b"0123456789".to_vec();
        let part = storage.plan_part(&attempt, 1, &plaintext).unwrap();
        storage.write_part(&part, plaintext.clone()).await.unwrap();
        let object = storage
            .seal_parts(
                "video/mp4",
                plaintext.len() as u64,
                &hex::encode(Sha256::digest(&plaintext)),
                vec![part],
            )
            .await
            .unwrap();
        (storage, object)
    }

    #[tokio::test]
    async fn full_and_partial_get_emit_exact_headers_and_bytes() {
        let (storage, object) = fixture().await;
        let slots = Arc::new(Semaphore::new(1));
        let permit = slots.clone().acquire_owned().await.unwrap();
        let response = media_response(
            &storage,
            &object,
            RequestedRange::Full,
            Method::GET,
            Some(permit),
            Some("recording.mov"),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[ACCEPT_RANGES], "bytes");
        assert_eq!(response.headers()[CONTENT_LENGTH], "10");
        assert_eq!(response.headers()[CONTENT_TYPE], "video/mp4");
        assert_eq!(response.headers()[CACHE_CONTROL], "private, no-store");
        assert!(
            response.headers()[CONTENT_DISPOSITION]
                .to_str()
                .unwrap()
                .starts_with("attachment; filename=\"recording.mov\"")
        );
        assert!(slots.try_acquire().is_err());
        let body = to_bytes(response.into_body(), 10).await.unwrap();
        assert_eq!(body, "0123456789");
        assert!(slots.try_acquire().is_ok());

        let permit = slots.clone().acquire_owned().await.unwrap();
        let response = media_response(
            &storage,
            &object,
            RequestedRange::Partial(3..=6),
            Method::GET,
            Some(permit),
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes 3-6/10");
        assert_eq!(response.headers()[CONTENT_LENGTH], "4");
        assert_eq!(to_bytes(response.into_body(), 4).await.unwrap(), "3456");
    }

    #[tokio::test]
    async fn range_head_has_get_headers_and_no_body() {
        let (storage, object) = fixture().await;
        let response = media_response(
            &storage,
            &object,
            RequestedRange::Partial(3..=6),
            Method::HEAD,
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes 3-6/10");
        assert_eq!(response.headers()[CONTENT_LENGTH], "4");
        assert!(to_bytes(response.into_body(), 1).await.unwrap().is_empty());
    }

    #[test]
    fn unsatisfiable_range_response_advertises_object_size() {
        use axum::response::IntoResponse as _;
        let response = ServiceError::range_not_satisfiable(10).into_response();
        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes */10");
        assert_eq!(response.headers()[ACCEPT_RANGES], "bytes");
    }

    #[tokio::test]
    async fn upload_admission_rejects_excess_and_times_out_a_stalled_body() {
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().acquire_owned().await.unwrap();
        let busy = buffer_upload_body(slots.clone(), Body::from("part"), Duration::from_millis(10))
            .await
            .unwrap_err();
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
        drop(held);

        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        let stalled = Body::from_stream(ReceiverStream::new(receiver));
        let timeout = buffer_upload_body(slots, stalled, Duration::from_millis(10))
            .await
            .unwrap_err();
        assert_eq!(timeout.status(), StatusCode::REQUEST_TIMEOUT);
        drop(sender);

        let too_large = buffer_upload_body(
            Arc::new(Semaphore::new(1)),
            Body::from(vec![0_u8; MAX_CHUNK_BYTES + 1]),
            Duration::from_secs(1),
        )
        .await
        .unwrap_err();
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
