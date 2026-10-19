use crate::{
    AppState,
    error::{media_metadata_error, media_not_found, media_storage_error},
    ranges::{RequestedRange, requested_range},
};
use axum::{
    Json,
    body::Body,
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
use scope_api_contract::{RequestAttachmentMediaTarget, RequestAttachmentPartReceiptResponse};
use scope_domain::requests::attachments::RequestMediaObject;
use scope_media_storage::{MAX_CHUNK_BYTES, MEDIA_PUT_TIMEOUT, WriteAttempt};
use scope_postgres::db::{RequestMediaObjectTarget, ReserveUploadPartResult, StorePartResult};
use scope_service_runtime::{http::ServiceError, random_hex, unix_now};
use serde::Deserialize;
use std::{sync::Arc, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_stream::StreamExt;

const UPLOAD_ADMISSION_WAIT: Duration = Duration::from_secs(30);
const READ_ADMISSION_WAIT: Duration = Duration::from_secs(10);
const UPLOAD_BODY_STALL: Duration = Duration::from_secs(20);
const UPLOAD_BODY_DEADLINE: Duration = Duration::from_secs(600);

#[derive(Deserialize)]
pub(crate) struct GrantQuery {
    grant: String,
}

pub(crate) async fn healthz() -> StatusCode {
    StatusCode::NO_CONTENT
}

pub(crate) async fn readyz(State(state): State<AppState>) -> Result<StatusCode, ServiceError> {
    state.metadata.admin().readiness_check().await?;
    state
        .storage
        .readiness_check()
        .await
        .map_err(media_storage_error)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn put_upload_part(
    State(state): State<AppState>,
    Path((upload_id, part_number)): Path<(String, u32)>,
    request: axum::extract::Request,
) -> Result<Json<RequestAttachmentPartReceiptResponse>, ServiceError> {
    let claims = state
        .verifier
        .verify_upload(request.headers(), &upload_id, unix_now()?)?;
    let _permit = admit(
        &state.upload_slots,
        UPLOAD_ADMISSION_WAIT,
        "media upload capacity is busy; retry the part",
    )
    .await?;
    let part_bytes = buffer_upload_body(request).await?;
    let attempt = WriteAttempt::new(&claims.attachment_id, "original", &claims.upload_id)
        .map_err(media_storage_error)?;
    let planned = state
        .storage
        .plan_part(&attempt, part_number, &part_bytes)
        .map_err(media_storage_error)?;
    let write_token = random_hex("mpw_", 16).map_err(|error| {
        ServiceError::internal(format!("media token generation failed: {error}"))
    })?;
    let write_started_at_unix = unix_now()?;
    let write_expires_at_unix = write_started_at_unix
        .checked_add(MEDIA_PUT_TIMEOUT.as_secs())
        .ok_or_else(|| ServiceError::internal("media part write expiry overflowed"))?;
    let reserved = match state
        .metadata
        .media()
        .reserve_upload_part(
            &claims.attachment_id,
            &claims.upload_id,
            &claims.uploader_user_id,
            planned,
            &write_token,
            write_started_at_unix,
            write_expires_at_unix,
        )
        .await
        .map_err(media_metadata_error)?
    {
        ReserveUploadPartResult::Write(part) => part,
        ReserveUploadPartResult::Stored(part) => return Ok(Json(part.receipt.into())),
        ReserveUploadPartResult::Busy => {
            return Err(ServiceError::unavailable(
                "media part is already being stored; retry",
            ));
        }
    };
    state
        .storage
        .write_part(&reserved, part_bytes)
        .await
        .map_err(media_storage_error)?;
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
        .await
        .map_err(media_metadata_error);
    match store_result {
        Ok(StorePartResult::Recorded | StorePartResult::AlreadyRecorded(_)) => {}
        Ok(StorePartResult::WriteLeaseLost) => {
            state
                .storage
                .delete_object_key(&reserved.object_key)
                .await
                .map_err(media_storage_error)?;
            return Err(ServiceError::conflict(
                "media part write lease was lost; retry the part",
            ));
        }
        Err(error) => return Err(error),
    }
    Ok(Json(reserved.receipt.into()))
}

async fn admit(
    slots: &Arc<Semaphore>,
    wait: Duration,
    busy: &'static str,
) -> Result<OwnedSemaphorePermit, ServiceError> {
    tokio::time::timeout(wait, slots.clone().acquire_owned())
        .await
        .map_err(|_| ServiceError::unavailable(busy))?
        .map_err(|_| ServiceError::internal("media admission pool is closed"))
}

async fn buffer_upload_body(request: axum::extract::Request) -> Result<Vec<u8>, ServiceError> {
    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok()?.parse::<usize>().ok())
        .unwrap_or(0);
    let mut stream = request.into_body().into_data_stream();
    let mut bytes = Vec::with_capacity(declared.min(MAX_CHUNK_BYTES));
    let timed_out = || ServiceError::request_timeout("media part body timed out");
    tokio::time::timeout(UPLOAD_BODY_DEADLINE, async {
        while let Some(frame) = tokio::time::timeout(UPLOAD_BODY_STALL, stream.next())
            .await
            .map_err(|_| timed_out())?
        {
            let frame =
                frame.map_err(|_| ServiceError::bad_request("media part body was interrupted"))?;
            if bytes.len() + frame.len() > MAX_CHUNK_BYTES {
                return Err(ServiceError::payload_too_large(
                    "media part exceeds the 8 MiB limit",
                ));
            }
            bytes.extend_from_slice(&frame);
        }
        Ok(bytes)
    })
    .await
    .map_err(|_| timed_out())?
}

pub(crate) async fn original(
    state: State<AppState>,
    path: Path<String>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
    method: Method,
) -> Result<Response, ServiceError> {
    serve_media(state.0, path.0, None, query.0.grant, headers, method).await
}

pub(crate) async fn derivative(
    state: State<AppState>,
    Path((attachment_id, derivative_id)): Path<(String, String)>,
    query: Query<GrantQuery>,
    headers: HeaderMap,
    method: Method,
) -> Result<Response, ServiceError> {
    serve_media(
        state.0,
        attachment_id,
        Some(derivative_id),
        query.0.grant,
        headers,
        method,
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
    let db_target = match derivative_id.as_deref() {
        Some(id) => RequestMediaObjectTarget::Derivative(id),
        None => RequestMediaObjectTarget::Original,
    };
    let (attachment, object) = state
        .metadata
        .media()
        .request_attachment_media_for_viewer(
            &claims.request_id,
            &attachment_id,
            claims.viewer_user_id.as_deref(),
            db_target,
        )
        .await
        .map_err(media_metadata_error)?
        .filter(|(attachment, _)| attachment.repository_id == claims.repository_id)
        .ok_or_else(media_not_found)?;
    let object = object.ok_or_else(media_not_found)?;
    let original_filename = derivative_id.is_none().then_some(attachment.filename);
    let range = requested_range(&headers, object.plaintext_bytes)?;
    let permit = if method == Method::GET && object.plaintext_bytes > 0 {
        Some(
            admit(
                &state.read_slots,
                READ_ADMISSION_WAIT,
                "media read capacity is busy; retry",
            )
            .await?,
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

async fn media_response(
    storage: &scope_media_storage::MediaStorage,
    object: &RequestMediaObject,
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
            .await
            .map_err(media_storage_error)?;
        let permit = permit.expect("GET read permit");
        Body::from_stream(stream.map(move |bytes| {
            let _held = &permit;
            bytes
        }))
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use bytes::Bytes;
    use scope_media_storage::WriteAttempt;
    use scope_storage::MemoryBackend;
    use sha2::{Digest, Sha256};
    use tokio_stream::wrappers::ReceiverStream;

    async fn fixture() -> (scope_media_storage::MediaStorage, RequestMediaObject) {
        let storage = scope_media_storage::MediaStorage::encrypted(
            Arc::new(MemoryBackend::default()),
            [21; 32],
            1,
        )
        .unwrap();
        let attempt = WriteAttempt::new("att_test", "original", "upload_test").unwrap();
        let plaintext = b"0123456789".to_vec();
        let part = storage.plan_part(&attempt, 1, &plaintext).unwrap();
        storage.write_part(&part, plaintext.clone()).await.unwrap();
        let object = scope_media_storage::seal_parts(
            "video/mp4",
            plaintext.len() as u64,
            &hex::encode(Sha256::digest(&plaintext)),
            vec![part],
        )
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

    #[tokio::test(start_paused = true)]
    async fn admission_waits_for_a_slot_before_reporting_busy() {
        let slots = Arc::new(Semaphore::new(1));
        let held = slots.clone().acquire_owned().await.unwrap();
        let release = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(held);
        });
        drop(admit(&slots, READ_ADMISSION_WAIT, "busy").await.unwrap());
        release.await.unwrap();

        let _held = slots.clone().acquire_owned().await.unwrap();
        let busy = admit(&slots, READ_ADMISSION_WAIT, "busy")
            .await
            .unwrap_err();
        assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    fn streamed_body(frames: Vec<(u64, &'static [u8])>) -> axum::extract::Request {
        let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, std::io::Error>>(1);
        tokio::spawn(async move {
            for (delay_secs, frame) in frames {
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                if sender.send(Ok(Bytes::from_static(frame))).await.is_err() {
                    return;
                }
            }
        });
        axum::extract::Request::new(Body::from_stream(ReceiverStream::new(receiver)))
    }

    #[tokio::test(start_paused = true)]
    async fn upload_bodies_fail_on_stalls_not_on_slow_steady_progress() {
        let slow = buffer_upload_body(streamed_body(vec![(15, b"part"); 6]))
            .await
            .unwrap();
        assert_eq!(slow, b"part".repeat(6));

        let stalled = buffer_upload_body(streamed_body(vec![(0, b"part"), (3600, b"late")]))
            .await
            .unwrap_err();
        assert_eq!(stalled.status(), StatusCode::REQUEST_TIMEOUT);

        let endless = buffer_upload_body(streamed_body(vec![(15, b"x"); 100]))
            .await
            .unwrap_err();
        assert_eq!(endless.status(), StatusCode::REQUEST_TIMEOUT);

        let too_large = buffer_upload_body(axum::extract::Request::new(Body::from(vec![
            0_u8;
            MAX_CHUNK_BYTES
                + 1
        ])))
        .await
        .unwrap_err();
        assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }
}
