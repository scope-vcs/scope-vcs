use scope_cache_domain::CacheDomainError;
use scope_service_runtime::http::{ErrorKind, ServiceError};

/// Cache domain rejections are caller-visible: the runner sent an unusable
/// digest or lease, or the repository is over its cache budget.
pub(crate) fn cache_domain_error(error: CacheDomainError) -> ServiceError {
    let kind = match error {
        CacheDomainError::RepositoryBudgetExceeded { .. } => ErrorKind::TooManyRequests,
        CacheDomainError::StaleUploadLease | CacheDomainError::UploadLeaseExpired => {
            ErrorKind::Conflict
        }
        _ => ErrorKind::BadRequest,
    };
    ServiceError::new(kind, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_and_lease_failures_keep_their_distinct_kinds() {
        assert_eq!(
            cache_domain_error(CacheDomainError::RepositoryBudgetExceeded {
                requested_bytes: 2,
                maximum_bytes: 1,
            })
            .kind(),
            ErrorKind::TooManyRequests
        );
        assert_eq!(
            cache_domain_error(CacheDomainError::StaleUploadLease).kind(),
            ErrorKind::Conflict
        );
        assert_eq!(
            cache_domain_error(CacheDomainError::InvalidDigest).kind(),
            ErrorKind::BadRequest
        );
    }
}
