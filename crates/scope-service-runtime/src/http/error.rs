use axum::{
    Json,
    http::{
        HeaderName, HeaderValue, StatusCode,
        header::{ACCEPT_RANGES, CONTENT_RANGE},
    },
    response::{IntoResponse, Response},
};
use scope_api_contract::{ErrorCode, ErrorResponse};

pub const INTERNAL_MESSAGE: &str = "Scope hit an internal error.";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    PayloadTooLarge,
    RangeNotSatisfiable,
    RequestTimeout,
    ServiceUnavailable,
    TooManyRequests,
    Unauthorized,
}

impl ErrorKind {
    pub fn status(self) -> StatusCode {
        match self {
            ErrorKind::BadRequest => StatusCode::BAD_REQUEST,
            ErrorKind::Conflict => StatusCode::CONFLICT,
            ErrorKind::Forbidden => StatusCode::FORBIDDEN,
            ErrorKind::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorKind::NotFound => StatusCode::NOT_FOUND,
            ErrorKind::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::RangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            ErrorKind::RequestTimeout => StatusCode::REQUEST_TIMEOUT,
            ErrorKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        }
    }

    pub const fn code(self) -> ErrorCode {
        match self {
            ErrorKind::BadRequest => ErrorCode::BadRequest,
            ErrorKind::Conflict => ErrorCode::Conflict,
            ErrorKind::Forbidden => ErrorCode::Forbidden,
            ErrorKind::Internal => ErrorCode::Internal,
            ErrorKind::NotFound => ErrorCode::NotFound,
            ErrorKind::PayloadTooLarge => ErrorCode::PayloadTooLarge,
            ErrorKind::RangeNotSatisfiable => ErrorCode::RangeNotSatisfiable,
            ErrorKind::RequestTimeout => ErrorCode::RequestTimeout,
            ErrorKind::ServiceUnavailable => ErrorCode::ServiceUnavailable,
            ErrorKind::TooManyRequests => ErrorCode::TooManyRequests,
            ErrorKind::Unauthorized => ErrorCode::Unauthorized,
        }
    }

    const fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::ServiceUnavailable | ErrorKind::TooManyRequests
        )
    }
}

#[derive(Clone, Debug)]
pub struct ServiceError {
    kind: ErrorKind,
    code: ErrorCode,
    message: String,
    headers: Vec<(HeaderName, HeaderValue)>,
}

impl ServiceError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: kind.code(),
            message: message.into(),
            headers: Vec::new(),
        }
    }

    pub fn internal(diagnostic: impl Into<String>) -> Self {
        let diagnostic = diagnostic.into();
        tracing::error!(%diagnostic, "service request failed");
        Self::new(ErrorKind::Internal, INTERNAL_MESSAGE)
    }

    pub fn range_not_satisfiable(size_bytes: u64) -> Self {
        let mut error = Self::new(
            ErrorKind::RangeNotSatisfiable,
            format!("requested range is outside the {size_bytes} byte object"),
        );
        error
            .headers
            .push((ACCEPT_RANGES, HeaderValue::from_static("bytes")));
        error.headers.push((
            CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes */{size_bytes}"))
                .expect("byte counts are printable"),
        ));
        error
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn status(&self) -> StatusCode {
        self.kind.status()
    }

    pub fn with_code(mut self, code: ErrorCode) -> Self {
        self.code = code;
        self
    }
}

macro_rules! message_errors {
    ($($name:ident => $kind:ident),+ $(,)?) => {
        impl ServiceError {$(
            pub fn $name(message: impl Into<String>) -> Self {
                Self::new(ErrorKind::$kind, message)
            }
        )+}
    };
}

message_errors! {
    bad_request => BadRequest,
    conflict => Conflict,
    forbidden => Forbidden,
    not_found => NotFound,
    payload_too_large => PayloadTooLarge,
    request_timeout => RequestTimeout,
    too_many_requests => TooManyRequests,
    unauthorized => Unauthorized,
    unavailable => ServiceUnavailable,
}

impl IntoResponse for ServiceError {
    fn into_response(self) -> Response {
        let mut body = ErrorResponse::new(self.code, self.message);
        body.retryable = self.kind.retryable();
        let mut response = (self.kind.status(), Json(body)).into_response();
        for (name, value) in self.headers {
            response.headers_mut().insert(name, value);
        }
        response
    }
}

#[cfg(feature = "postgres")]
pub fn postgres_error_kind(kind: scope_postgres::error::PostgresErrorKind) -> ErrorKind {
    use scope_postgres::error::PostgresErrorKind;
    match kind {
        PostgresErrorKind::AttachmentUploadExpired | PostgresErrorKind::Conflict => {
            ErrorKind::Conflict
        }
        PostgresErrorKind::InvalidInput => ErrorKind::BadRequest,
        PostgresErrorKind::PermissionDenied => ErrorKind::Forbidden,
        PostgresErrorKind::Internal => ErrorKind::Internal,
        PostgresErrorKind::NotFound => ErrorKind::NotFound,
        PostgresErrorKind::Unavailable => ErrorKind::ServiceUnavailable,
        PostgresErrorKind::ResourceExhausted => ErrorKind::TooManyRequests,
        PostgresErrorKind::Unauthenticated => ErrorKind::Unauthorized,
    }
}

#[cfg(feature = "postgres")]
impl From<scope_postgres::error::PostgresError> for ServiceError {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        use scope_postgres::error::PostgresErrorKind;

        let expired = error.kind == PostgresErrorKind::AttachmentUploadExpired;
        let error = match postgres_error_kind(error.kind) {
            ErrorKind::Internal => Self::internal(error.message),
            kind => Self::new(kind, error.message),
        };
        if expired {
            error.with_code(ErrorCode::AttachmentUploadExpired)
        } else {
            error
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    async fn body_json(error: ServiceError) -> (StatusCode, serde_json::Value) {
        let response = error.into_response();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn renders_the_contract_body_for_a_caller_visible_failure() {
        let response = ServiceError::conflict("cache upload lease is stale").into_response();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let bytes = to_bytes(response.into_body(), 16 * 1024).await.unwrap();
        assert_eq!(
            std::str::from_utf8(&bytes).unwrap(),
            r#"{"code":"conflict","message":"cache upload lease is stale","retryable":false}"#
        );
    }

    #[tokio::test]
    async fn temporary_failures_are_marked_retryable() {
        let (status, body) = body_json(ServiceError::unavailable("retry the part")).await;

        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["code"], "service_unavailable");
        assert_eq!(body["retryable"], true);
    }

    #[tokio::test]
    async fn internal_diagnostics_never_reach_the_body() {
        let (status, body) = body_json(ServiceError::internal(
            "s3 endpoint https://internal timed out",
        ))
        .await;

        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["code"], "internal");
        assert_eq!(body["message"], INTERNAL_MESSAGE);
    }

    #[tokio::test]
    async fn an_unsatisfiable_range_advertises_the_object_size() {
        let response = ServiceError::range_not_satisfiable(10).into_response();

        assert_eq!(response.status(), StatusCode::RANGE_NOT_SATISFIABLE);
        assert_eq!(response.headers()[CONTENT_RANGE], "bytes */10");
        assert_eq!(response.headers()[ACCEPT_RANGES], "bytes");
    }
}
