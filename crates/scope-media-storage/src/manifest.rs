use crate::{MAX_CHUNK_BYTES, MediaStorageError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteAttempt {
    pub attachment_id: String,
    pub object_name: String,
    pub attempt_id: String,
}

impl WriteAttempt {
    pub fn new(
        attachment_id: impl Into<String>,
        object_name: impl Into<String>,
        attempt_id: impl Into<String>,
    ) -> Result<Self, MediaStorageError> {
        let attempt = Self {
            attachment_id: attachment_id.into(),
            object_name: object_name.into(),
            attempt_id: attempt_id.into(),
        };
        crate::keys::validate_write_attempt(&attempt)?;
        Ok(attempt)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StagedMediaPart {
    pub part_number: u32,
    pub size_bytes: u64,
    pub sha256: String,
    pub object_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MediaChunk {
    pub part_number: u32,
    pub plaintext_offset: u64,
    pub plaintext_bytes: u64,
    pub sha256: String,
    pub object_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MediaObject {
    pub media_type: String,
    pub plaintext_bytes: u64,
    pub sha256: String,
    pub chunks: Vec<MediaChunk>,
}

impl MediaObject {
    pub fn new(
        media_type: impl Into<String>,
        plaintext_bytes: u64,
        sha256: impl Into<String>,
        chunks: Vec<MediaChunk>,
    ) -> Result<Self, MediaStorageError> {
        let object = Self {
            media_type: media_type.into(),
            plaintext_bytes,
            sha256: sha256.into(),
            chunks,
        };
        object.validate()?;
        Ok(object)
    }

    pub(crate) fn validate(&self) -> Result<(), MediaStorageError> {
        if self.media_type.is_empty() || self.media_type.len() > 255 {
            return Err(MediaStorageError::integrity(
                "media manifest contains an invalid content type",
            ));
        }
        validate_digest("media object", &self.sha256)?;
        let mut offset = 0_u64;
        let mut object_keys = BTreeSet::new();
        for (index, chunk) in self.chunks.iter().enumerate() {
            let expected_part = u32::try_from(index + 1)
                .map_err(|_| MediaStorageError::integrity("media manifest has too many chunks"))?;
            if chunk.part_number != expected_part || chunk.plaintext_offset != offset {
                return Err(MediaStorageError::integrity(
                    "media manifest chunks are not contiguous",
                ));
            }
            if chunk.plaintext_bytes == 0
                || chunk.plaintext_bytes > MAX_CHUNK_BYTES as u64
                || (index + 1 < self.chunks.len()
                    && chunk.plaintext_bytes != MAX_CHUNK_BYTES as u64)
            {
                return Err(MediaStorageError::integrity(
                    "media manifest contains an invalid chunk size",
                ));
            }
            validate_digest("media chunk", &chunk.sha256)?;
            if !chunk.object_key.starts_with("media/v1/staged/")
                || !object_keys.insert(&chunk.object_key)
            {
                return Err(MediaStorageError::integrity(
                    "media manifest contains an invalid or duplicate chunk key",
                ));
            }
            offset = offset.checked_add(chunk.plaintext_bytes).ok_or_else(|| {
                MediaStorageError::integrity("media manifest plaintext size overflowed")
            })?;
        }
        if offset != self.plaintext_bytes || (offset == 0 && !self.chunks.is_empty()) {
            return Err(MediaStorageError::integrity(
                "media manifest plaintext size does not match its chunks",
            ));
        }
        Ok(())
    }
}

pub(crate) fn validate_digest(label: &str, digest: &str) -> Result<(), MediaStorageError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(MediaStorageError::integrity(format!(
            "{label} has an invalid SHA-256 digest"
        )));
    }
    Ok(())
}
