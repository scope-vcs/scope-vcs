use axum::http::StatusCode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorKind {
    BadRequest,
    Conflict,
    Forbidden,
    Internal,
    NotFound,
    NotImplemented,
    PayloadTooLarge,
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
            ErrorKind::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            ErrorKind::PayloadTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            ErrorKind::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorKind::TooManyRequests => StatusCode::TOO_MANY_REQUESTS,
            ErrorKind::Unauthorized => StatusCode::UNAUTHORIZED,
        }
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
