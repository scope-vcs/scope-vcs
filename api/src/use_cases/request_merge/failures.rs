use crate::error::{ApiError, ErrorKind};

/// Why a merge attempt did not commit. Callers can retry transient failures without
/// mistaking every conflict-shaped API error for a Git content conflict.
pub(crate) enum RequestMergeFailure {
    Other(ApiError),
    MergeConflict(ApiError),
    RequestBranchMissing(ApiError),
}

impl RequestMergeFailure {
    pub(super) fn public_range(error: ApiError) -> Self {
        // These conflicts require a new request push. Retrying the authorized
        // revision cannot repair its ancestry or public path restrictions.
        if error.kind == ErrorKind::Conflict {
            Self::MergeConflict(error)
        } else {
            Self::Other(error)
        }
    }

    pub(crate) fn error(&self) -> &ApiError {
        match self {
            Self::Other(error) | Self::MergeConflict(error) | Self::RequestBranchMissing(error) => {
                error
            }
        }
    }

    pub(super) fn into_api_error(self) -> ApiError {
        match self {
            Self::Other(error) | Self::MergeConflict(error) | Self::RequestBranchMissing(error) => {
                error
            }
        }
    }
}

impl From<ApiError> for RequestMergeFailure {
    fn from(error: ApiError) -> Self {
        Self::Other(error)
    }
}

impl From<scope_postgres::error::PostgresError> for RequestMergeFailure {
    fn from(error: scope_postgres::error::PostgresError) -> Self {
        Self::Other(error.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_range_conflicts_stop_but_infrastructure_failures_retry() {
        for error in [
            ApiError::conflict("public main advanced"),
            ApiError::protected_paths(vec![".scope/RULES.md".to_string()]),
        ] {
            assert!(matches!(
                RequestMergeFailure::public_range(error),
                RequestMergeFailure::MergeConflict(_)
            ));
        }
        for error in [
            ApiError::internal_message("git process failed"),
            ApiError::infrastructure_unavailable("object storage unavailable"),
            ApiError::too_many_requests("storage throttled"),
        ] {
            assert!(matches!(
                RequestMergeFailure::public_range(error),
                RequestMergeFailure::Other(_)
            ));
        }
    }
}
