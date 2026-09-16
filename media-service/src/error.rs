use scope_media_storage::{MediaStorageError, MediaStorageErrorKind};
use scope_postgres::error::{PostgresError, PostgresErrorKind};
use scope_service_runtime::http::ServiceError;

/// The gateway never confirms which attachments exist: every unreachable
/// object, whatever the reason, reads the same to a caller.
pub(crate) fn media_not_found() -> ServiceError {
    ServiceError::not_found("media object not found")
}

/// The gateway never confirms which attachments exist: a metadata miss reads
/// the same as any other unreachable object.
pub(crate) fn media_metadata_error(error: PostgresError) -> ServiceError {
    if error.kind == PostgresErrorKind::NotFound {
        media_not_found()
    } else {
        ServiceError::from(error)
    }
}

pub(crate) fn media_storage_error(error: MediaStorageError) -> ServiceError {
    match error.kind {
        MediaStorageErrorKind::CapacityExhausted => ServiceError::too_many_requests(error.message),
        MediaStorageErrorKind::InvalidInput => ServiceError::bad_request(error.message),
        MediaStorageErrorKind::NotFound => media_not_found(),
        MediaStorageErrorKind::ServiceUnavailable => ServiceError::unavailable(error.message),
        MediaStorageErrorKind::Integrity | MediaStorageErrorKind::Internal => {
            ServiceError::internal(error.message)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_service_runtime::http::ErrorKind;

    #[tokio::test]
    async fn metadata_misses_read_like_every_other_unreachable_object() {
        use axum::{body::to_bytes, http::StatusCode, response::IntoResponse};

        let error = media_metadata_error(PostgresError::not_found(
            "request attachment 42 is not visible to viewer 7",
        ));
        let response = error.into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(
            body,
            br#"{"code":"not_found","message":"media object not found","retryable":false}"#
                .as_slice()
        );
    }

    #[test]
    fn storage_failures_keep_their_caller_visible_kinds() {
        let kinds = [
            (
                MediaStorageErrorKind::CapacityExhausted,
                ErrorKind::TooManyRequests,
            ),
            (MediaStorageErrorKind::InvalidInput, ErrorKind::BadRequest),
            (MediaStorageErrorKind::NotFound, ErrorKind::NotFound),
            (
                MediaStorageErrorKind::ServiceUnavailable,
                ErrorKind::ServiceUnavailable,
            ),
            (MediaStorageErrorKind::Integrity, ErrorKind::Internal),
            (MediaStorageErrorKind::Internal, ErrorKind::Internal),
        ];
        for (storage_kind, expected) in kinds {
            let error = media_storage_error(MediaStorageError {
                kind: storage_kind,
                message: "object key /srv/media/private".to_string(),
            });
            assert_eq!(error.kind(), expected, "{storage_kind:?}");
        }
    }
}
