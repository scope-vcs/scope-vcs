use crate::MediaStorageError;
use scope_domain::requests::attachments::RequestMediaObject;

pub(crate) const STAGED_KEY_PREFIX: &str = "media/v1/staged/";

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

pub(crate) fn validate_object(object: &RequestMediaObject) -> Result<(), MediaStorageError> {
    object
        .validate()
        .map_err(|error| MediaStorageError::integrity(error.message))?;
    if object
        .chunks
        .iter()
        .any(|chunk| !chunk.object_key.starts_with(STAGED_KEY_PREFIX))
    {
        return Err(MediaStorageError::integrity(
            "media manifest contains an invalid or duplicate chunk key",
        ));
    }
    Ok(())
}
