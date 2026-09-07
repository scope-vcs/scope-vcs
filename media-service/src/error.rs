use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use scope_media_storage::{MediaStorageError, MediaStorageErrorKind};
use scope_postgres::error::{PostgresError, PostgresErrorKind};
use serde::Serialize;

#[derive(Debug)]
pub(crate) struct ServiceError {
    status: StatusCode,
    message: String,
    content_range: Option<String>,
}

#[derive(Serialize)]
struct ErrorBody<'a> {
    error: &'a str,
}

impl ServiceError {
    pub(crate) fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub(crate) fn payload_too_large(message: impl Into<String>) -> Self {
        Self::new(StatusCode::PAYLOAD_TOO_LARGE, message)
    }

    pub(crate) fn request_timeout(message: impl Into<String>) -> Self {
        Self::new(StatusCode::REQUEST_TIMEOUT, message)
    }

    pub(crate) fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub(crate) fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }

    pub(crate) fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "media object not found")
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }

    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self::new(StatusCode::SERVICE_UNAVAILABLE, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        let message = message.into();
        tracing::error!(error = %message, "media service internal error");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "media service failed")
    }

    pub(crate) fn range_not_satisfiable(size_bytes: u64) -> Self {
        let mut error = Self::new(
            StatusCode::RANGE_NOT_SATISFIABLE,
            format!("requested range is outside the {size_bytes} byte media object"),
        );
        error.content_range = Some(format!("bytes */{size_bytes}"));
        error
    }

    #[cfg(test)]
    pub(crate) fn status(&self) -> StatusCode {
        self.status
    }

    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
            content_range: None,
        }
    }
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let mut response = (
            self.status,
            Json(ErrorBody {
                error: &self.message,
            }),
        )
            .into_response();
        if let Some(content_range) = self.content_range {
            response.headers_mut().insert(
                axum::http::header::CONTENT_RANGE,
                content_range.parse().expect("valid content range"),
            );
            response.headers_mut().insert(
                axum::http::header::ACCEPT_RANGES,
                axum::http::HeaderValue::from_static("bytes"),
            );
        }
        response
    }
}

impl From<PostgresError> for ServiceError {
    fn from(error: PostgresError) -> Self {
        match error.kind {
            PostgresErrorKind::InvalidInput => Self::bad_request(error.message),
            PostgresErrorKind::AttachmentUploadExpired | PostgresErrorKind::Conflict => {
                Self::conflict(error.message)
            }
            PostgresErrorKind::PermissionDenied => Self::forbidden(error.message),
            PostgresErrorKind::NotFound => Self::not_found(),
            PostgresErrorKind::ResourceExhausted => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, error.message)
            }
            PostgresErrorKind::Unauthenticated => Self::unauthorized(error.message),
            PostgresErrorKind::Unavailable => Self::unavailable(error.message),
            PostgresErrorKind::Internal => Self::internal(error.message),
        }
    }
}

impl From<MediaStorageError> for ServiceError {
    fn from(error: MediaStorageError) -> Self {
        match error.kind {
            MediaStorageErrorKind::CapacityExhausted => {
                Self::new(StatusCode::TOO_MANY_REQUESTS, error.message)
            }
            MediaStorageErrorKind::InvalidInput => Self::bad_request(error.message),
            MediaStorageErrorKind::NotFound => Self::not_found(),
            MediaStorageErrorKind::ServiceUnavailable => Self::unavailable(error.message),
            MediaStorageErrorKind::Integrity | MediaStorageErrorKind::Internal => {
                Self::internal(error.message)
            }
        }
    }
}
