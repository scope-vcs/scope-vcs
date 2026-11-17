#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostgresErrorKind {
    AttachmentUploadExpired,
    InvalidInput,
    Conflict,
    RepositoryMoved,
    PermissionDenied,
    Internal,
    Deadlock,
    SerializationFailure,
    NotFound,
    Unavailable,
    ResourceExhausted,
    Unauthenticated,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct PostgresError {
    pub kind: PostgresErrorKind,
    pub message: String,
}

macro_rules! message_errors {
    ($($name:ident => $kind:ident),+ $(,)?) => {$(
        pub fn $name(message: impl Into<String>) -> Self {
            Self::new(PostgresErrorKind::$kind, message)
        }
    )+};
}

impl PostgresError {
    pub fn invalid_input(error: impl std::fmt::Display) -> Self {
        Self::new(PostgresErrorKind::InvalidInput, error.to_string())
    }

    pub fn internal(error: impl std::error::Error + 'static) -> Self {
        Self::new(failure_kind(&error), error.to_string())
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self.kind,
            PostgresErrorKind::Deadlock | PostgresErrorKind::SerializationFailure
        )
    }

    message_errors! {
        attachment_upload_expired => AttachmentUploadExpired,
        permission_denied => PermissionDenied,
        conflict => Conflict,
        repository_moved => RepositoryMoved,
        resource_exhausted => ResourceExhausted,
        unauthenticated => Unauthenticated,
        not_found => NotFound,
        internal_message => Internal,
        unavailable => Unavailable,
    }

    fn new(kind: PostgresErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<scope_domain::error::DomainError> for PostgresError {
    fn from(error: scope_domain::error::DomainError) -> Self {
        use scope_domain::error::DomainErrorKind;

        let kind = match error.kind {
            DomainErrorKind::InvalidInput => PostgresErrorKind::InvalidInput,
            DomainErrorKind::Conflict => PostgresErrorKind::Conflict,
            DomainErrorKind::Forbidden => PostgresErrorKind::PermissionDenied,
            DomainErrorKind::AuthenticationFailed => PostgresErrorKind::Unauthenticated,
            DomainErrorKind::RateLimited => PostgresErrorKind::ResourceExhausted,
            DomainErrorKind::NotFound => PostgresErrorKind::NotFound,
            DomainErrorKind::InvariantViolation => PostgresErrorKind::Internal,
        };
        Self::new(kind, error.message)
    }
}

impl From<sea_orm::DbErr> for PostgresError {
    fn from(error: sea_orm::DbErr) -> Self {
        Self::internal(error)
    }
}

fn failure_kind(error: &(dyn std::error::Error + 'static)) -> PostgresErrorKind {
    let sqlx_error = match error.downcast_ref::<sea_orm::DbErr>() {
        Some(
            sea_orm::DbErr::Conn(sea_orm::RuntimeErr::SqlxError(error))
            | sea_orm::DbErr::Exec(sea_orm::RuntimeErr::SqlxError(error))
            | sea_orm::DbErr::Query(sea_orm::RuntimeErr::SqlxError(error)),
        ) => Some(error.as_ref()),
        Some(_) => None,
        None => error.downcast_ref::<sqlx::Error>(),
    };
    match sqlx_error
        .and_then(sqlx::Error::as_database_error)
        .and_then(|error| error.code())
        .as_deref()
    {
        Some("40P01") => PostgresErrorKind::Deadlock,
        Some("40001") => PostgresErrorKind::SerializationFailure,
        Some(_) | None => PostgresErrorKind::Internal,
    }
}
