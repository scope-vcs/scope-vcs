use crate::error::DomainError;

pub(super) fn is_git_oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn validate_git_oid(label: &str, value: &str) -> Result<(), DomainError> {
    if !is_git_oid(value) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-1 hex digest"
        )));
    }
    Ok(())
}

pub(super) fn validate_sha256_hash(label: &str, value: &str) -> Result<(), DomainError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(format!(
            "{label} must be a SHA-256 hex digest"
        )));
    }
    Ok(())
}

pub(super) fn required(label: &str, value: String) -> Result<String, DomainError> {
    if value.trim().is_empty() {
        Err(DomainError::invalid_input(format!("{label} is required")))
    } else {
        Ok(value)
    }
}

pub(super) fn is_kebab_name(name: &str, max_bytes: usize) -> bool {
    !name.is_empty()
        && name.len() <= max_bytes
        && !name.starts_with('-')
        && !name.ends_with('-')
        && !name.contains("--")
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}
