use crate::error::{ApiError, ErrorKind};

/// Why a merge attempt did not commit. Callers can retry transient failures without
/// mistaking every conflict-shaped API error for a Git content conflict.
pub(crate) enum RequestMergeFailure {
    Rejected(ApiError),
    Retryable(ApiError),
    MergeConflict(ApiError),
    RequestBranchMissing(ApiError),
}

impl RequestMergeFailure {
    pub(crate) fn error(&self) -> &ApiError {
        match self {
            Self::Rejected(error)
            | Self::Retryable(error)
            | Self::MergeConflict(error)
            | Self::RequestBranchMissing(error) => error,
        }
    }

    pub(super) fn into_api_error(self) -> ApiError {
        match self {
            Self::Rejected(error)
            | Self::Retryable(error)
            | Self::MergeConflict(error)
            | Self::RequestBranchMissing(error) => error,
        }
    }

    fn classify(error: ApiError) -> Self {
        if matches!(
            error.kind,
            ErrorKind::Internal | ErrorKind::ServiceUnavailable | ErrorKind::TooManyRequests
        ) {
            Self::Retryable(error)
        } else {
            Self::Rejected(error)
        }
    }
}

impl From<ApiError> for RequestMergeFailure {
    fn from(error: ApiError) -> Self {
        Self::classify(error)
    }
}

impl From<scope_postgres::error::PostgresError> for RequestMergeFailure {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        Self::classify(error.into())
    }
}
