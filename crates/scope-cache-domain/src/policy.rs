use crate::CacheDomainError;

pub const GIB: u64 = 1024 * 1024 * 1024;
pub const MAX_CACHE_OBJECT_BYTES: u64 = GIB;
pub const MAX_REPOSITORY_CACHE_BYTES: u64 = 5 * GIB;
pub const CACHE_REFERENCE_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const UPLOAD_LEASE_SECONDS: u64 = 30 * 60;
pub const DELETION_GRACE_SECONDS: u64 = 60 * 60;

pub fn validate_object_size(size_bytes: u64) -> Result<(), CacheDomainError> {
    if size_bytes == 0 {
        return Err(CacheDomainError::EmptyObject);
    }
    if size_bytes > MAX_CACHE_OBJECT_BYTES {
        return Err(CacheDomainError::ObjectTooLarge {
            actual_bytes: size_bytes,
            maximum_bytes: MAX_CACHE_OBJECT_BYTES,
        });
    }
    Ok(())
}

pub fn validate_repository_growth(
    stored_bytes: u64,
    additional_bytes: u64,
) -> Result<(), CacheDomainError> {
    let requested_bytes = stored_bytes
        .checked_add(additional_bytes)
        .ok_or(CacheDomainError::ByteCountOverflow)?;
    if requested_bytes > MAX_REPOSITORY_CACHE_BYTES {
        return Err(CacheDomainError::RepositoryBudgetExceeded {
            requested_bytes,
            maximum_bytes: MAX_REPOSITORY_CACHE_BYTES,
        });
    }
    Ok(())
}

pub(crate) fn reference_expiry(now_unix: u64) -> Result<u64, CacheDomainError> {
    now_unix
        .checked_add(CACHE_REFERENCE_TTL_SECONDS)
        .ok_or(CacheDomainError::TimestampOverflow)
}

pub(crate) fn upload_expiry(now_unix: u64) -> Result<u64, CacheDomainError> {
    now_unix
        .checked_add(UPLOAD_LEASE_SECONDS)
        .ok_or(CacheDomainError::TimestampOverflow)
}

pub(crate) fn deletion_eligible_at(now_unix: u64) -> Result<u64, CacheDomainError> {
    now_unix
        .checked_add(DELETION_GRACE_SECONDS)
        .ok_or(CacheDomainError::TimestampOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_limits_accept_the_boundary_and_reject_growth_past_it() {
        assert_eq!(validate_object_size(MAX_CACHE_OBJECT_BYTES), Ok(()));
        assert!(matches!(
            validate_object_size(MAX_CACHE_OBJECT_BYTES + 1),
            Err(CacheDomainError::ObjectTooLarge { .. })
        ));
        assert_eq!(
            validate_repository_growth(
                MAX_REPOSITORY_CACHE_BYTES - MAX_CACHE_OBJECT_BYTES,
                MAX_CACHE_OBJECT_BYTES,
            ),
            Ok(())
        );
        assert!(matches!(
            validate_repository_growth(MAX_REPOSITORY_CACHE_BYTES, 1),
            Err(CacheDomainError::RepositoryBudgetExceeded { .. })
        ));
    }
}
