use super::{
    RequestAttachment, RequestAttachmentFailure, RequestAttachmentFailureCode,
    RequestAttachmentKind, rules::validate_processing_failure,
};
use crate::error::DomainError;

pub const REQUEST_ATTACHMENT_MAX_PROCESSING_ATTEMPTS: u32 = 4;

pub fn processing_attempts_exhausted(attempt: u32) -> bool {
    attempt >= REQUEST_ATTACHMENT_MAX_PROCESSING_ATTEMPTS
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestAttachmentProcessingLane {
    Photo,
    General,
}

impl RequestAttachmentProcessingLane {
    pub fn admitted_kind(self) -> Option<RequestAttachmentKind> {
        match self {
            Self::Photo => Some(RequestAttachmentKind::Photo),
            Self::General => None,
        }
    }
}

pub fn fail_interrupted_processing(
    attachment: &RequestAttachment,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    validate_processing_failure(
        attachment,
        RequestAttachmentFailure {
            code: RequestAttachmentFailureCode::Internal,
            message: "Media processing stopped unexpectedly. Retry processing.".to_owned(),
            retryable: true,
        },
        now_unix,
    )
}
