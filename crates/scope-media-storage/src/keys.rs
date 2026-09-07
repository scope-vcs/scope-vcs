use crate::MediaStorageError;

pub(crate) fn staged_chunk_key(
    attempt: &crate::WriteAttempt,
    part_number: u32,
) -> Result<String, MediaStorageError> {
    validate_segment("attachment ID", &attempt.attachment_id)?;
    validate_segment("object name", &attempt.object_name)?;
    validate_segment("attempt ID", &attempt.attempt_id)?;
    if part_number == 0 {
        return Err(MediaStorageError::invalid("part numbers start at one"));
    }
    Ok(format!(
        "media/v1/staged/{}/{}/{}/parts/{part_number:08}-{}",
        attempt.attachment_id,
        attempt.object_name,
        attempt.attempt_id,
        random_suffix()?
    ))
}

pub(crate) fn validate_write_attempt(
    attempt: &crate::WriteAttempt,
) -> Result<(), MediaStorageError> {
    validate_segment("attachment ID", &attempt.attachment_id)?;
    validate_segment("object name", &attempt.object_name)?;
    validate_segment("attempt ID", &attempt.attempt_id)
}

fn random_suffix() -> Result<String, MediaStorageError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| MediaStorageError::internal(format!("media key nonce failed: {error}")))?;
    Ok(hex::encode(bytes))
}

fn validate_segment(label: &str, value: &str) -> Result<(), MediaStorageError> {
    if value.is_empty()
        || value.len() > 160
        || matches!(value, "." | "..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(MediaStorageError::invalid(format!(
            "{label} is not a valid storage key segment"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attempt_keys_are_unique_and_cannot_escape_the_namespace() {
        let attempt = crate::WriteAttempt::new("att_1", "original", "lease_1").unwrap();
        let first = staged_chunk_key(&attempt, 1).unwrap();
        let second = staged_chunk_key(&attempt, 1).unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with("media/v1/staged/att_1/original/lease_1/parts/00000001-"));
        assert!(crate::WriteAttempt::new("../att", "original", "lease_1").is_err());
    }
}
