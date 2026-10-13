pub(crate) fn generate_prefixed_id(prefix: &str) -> Result<String, crate::error::ApiError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|error| {
        crate::error::ApiError::internal_message(format!("failed to generate identifier: {error}"))
    })?;
    Ok(format!("{prefix}_{}", hex::encode(bytes)))
}
