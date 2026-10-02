use super::validation::required;
use crate::error::DomainError;

pub const MAX_RUN_LOG_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_RUN_LOG_BYTES_PER_ATTEMPT: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunLogChunk {
    pub attempt_id: String,
    pub step_index: u32,
    pub sequence: u64,
    pub text: String,
    pub created_at_unix: u64,
}

impl RunLogChunk {
    pub fn new(
        attempt_id: impl Into<String>,
        step_index: u32,
        sequence: u64,
        text: impl Into<String>,
        created_at_unix: u64,
    ) -> Result<Self, DomainError> {
        let attempt_id = required("run log attempt id", attempt_id.into())?;
        if sequence == 0 {
            return Err(DomainError::invalid_input(
                "run log sequence must be positive",
            ));
        }
        let text = text.into();
        if text.is_empty() {
            return Err(DomainError::invalid_input("run log text is required"));
        }
        if text.len() > MAX_RUN_LOG_CHUNK_BYTES {
            return Err(DomainError::invalid_input(format!(
                "run log chunk cannot exceed {MAX_RUN_LOG_CHUNK_BYTES} bytes"
            )));
        }
        if text.contains('\0') {
            return Err(DomainError::invalid_input(
                "run log text cannot contain NUL characters",
            ));
        }
        Ok(Self {
            attempt_id,
            step_index,
            sequence,
            text,
            created_at_unix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DomainErrorKind;

    #[test]
    fn run_log_text_rejects_nul_but_accepts_replacement_characters() {
        let error = RunLogChunk::new("attempt-1", 0, 1, "before\0after", 1).unwrap_err();
        assert_eq!(error.kind, DomainErrorKind::InvalidInput);

        let chunk = RunLogChunk::new("attempt-1", 0, 1, "before\u{FFFD}after", 1).unwrap();
        assert_eq!(chunk.text, "before\u{FFFD}after");
    }
}
