use scope_object_store::{ObjectStoreError, ObjectStoreErrorKind};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MediaStorageErrorKind {
    CapacityExhausted,
    Integrity,
    Internal,
    InvalidInput,
    NotFound,
    ServiceUnavailable,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct MediaStorageError {
    pub kind: MediaStorageErrorKind,
    pub message: String,
}

impl MediaStorageError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::new(MediaStorageErrorKind::InvalidInput, message)
    }

    pub(crate) fn integrity(message: impl Into<String>) -> Self {
        Self::new(MediaStorageErrorKind::Integrity, message)
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::new(MediaStorageErrorKind::Internal, message)
    }

    pub(crate) fn new(kind: MediaStorageErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

impl From<ObjectStoreError> for MediaStorageError {
    fn from(error: ObjectStoreError) -> Self {
        let kind = match error.kind {
            ObjectStoreErrorKind::CapacityExhausted => MediaStorageErrorKind::CapacityExhausted,
            ObjectStoreErrorKind::Integrity => MediaStorageErrorKind::Integrity,
            ObjectStoreErrorKind::Internal => MediaStorageErrorKind::Internal,
            ObjectStoreErrorKind::NotFound => MediaStorageErrorKind::NotFound,
            ObjectStoreErrorKind::PayloadTooLarge => MediaStorageErrorKind::Integrity,
            ObjectStoreErrorKind::ServiceUnavailable => MediaStorageErrorKind::ServiceUnavailable,
        };
        Self::new(kind, error.message)
    }
}
