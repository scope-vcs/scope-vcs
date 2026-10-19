use crate::requests::RequestPermissions;
use serde::{Deserialize, Serialize};

pub const REQUEST_ATTACHMENT_PHOTO_MEDIA_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/heic",
    "image/heif",
];
pub const REQUEST_MEDIA_CHUNK_BYTES: u64 = 8 * 1024 * 1024;
pub const REQUEST_ATTACHMENT_VIDEO_MEDIA_TYPES: &[&str] =
    &["video/mp4", "video/quicktime", "video/webm"];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RequestAttachmentKind {
    Photo,
    Video,
}

impl RequestAttachmentKind {
    pub(crate) fn from_media_type(media_type: &str) -> Option<Self> {
        let media_type = media_type.trim();
        if REQUEST_ATTACHMENT_PHOTO_MEDIA_TYPES
            .iter()
            .any(|accepted| media_type.eq_ignore_ascii_case(accepted))
        {
            Some(Self::Photo)
        } else if REQUEST_ATTACHMENT_VIDEO_MEDIA_TYPES
            .iter()
            .any(|accepted| media_type.eq_ignore_ascii_case(accepted))
        {
            Some(Self::Video)
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RequestAttachmentState {
    Prepared,
    Uploaded,
    Processing,
    Ready,
    Failed,
    Rejected,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RequestAttachmentTarget {
    Description,
    Discussion { discussion_id: Option<String> },
    Reply { discussion_id: String },
}

impl RequestAttachmentTarget {
    pub fn writable_with(&self, permissions: &RequestPermissions) -> bool {
        match self {
            Self::Description => permissions.can_edit_identity,
            Self::Discussion { .. } => permissions.can_open_discussion,
            Self::Reply { .. } => permissions.can_reply_to_discussion,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum RequestAttachmentBindingTarget {
    Description,
    Discussion {
        discussion_id: String,
    },
    Reply {
        discussion_id: String,
        reply_id: String,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RequestAttachmentFailureCode {
    InvalidMedia,
    CorruptMedia,
    UnsupportedMedia,
    MediaLimitExceeded,
    CodecFailed,
    StorageUnavailable,
    Internal,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentFailure {
    pub code: RequestAttachmentFailureCode,
    pub message: String,
    pub retryable: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentImageMetadata {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentVideoMetadata {
    pub width: u32,
    pub height: u32,
    pub duration_millis: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub enum RequestAttachmentDerivativeKind {
    ImagePreview,
    VideoPlayback,
    VideoPoster,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentStoredObject {
    pub object_key: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentDerivative {
    pub id: String,
    pub kind: RequestAttachmentDerivativeKind,
    pub media_type: String,
    pub object: RequestAttachmentStoredObject,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachment {
    pub id: String,
    pub repository_id: String,
    pub request_id: String,
    pub uploader_user_id: String,
    pub upload_id: String,
    pub operation_id: String,
    pub target: RequestAttachmentTarget,
    pub filename: String,
    pub declared_media_type: String,
    pub detected_media_type: Option<String>,
    pub kind: RequestAttachmentKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub state: RequestAttachmentState,
    pub original: Option<RequestAttachmentStoredObject>,
    pub original_validated_at_unix: Option<u64>,
    pub failure: Option<RequestAttachmentFailure>,
    pub image: Option<RequestAttachmentImageMetadata>,
    pub video: Option<RequestAttachmentVideoMetadata>,
    pub derivatives: Vec<RequestAttachmentDerivative>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
    pub upload_expires_at_unix: u64,
}

impl RequestAttachment {
    pub fn original_is_grantable(&self) -> bool {
        self.original.is_some()
            && self.original_validated_at_unix.is_some()
            && !matches!(
                self.state,
                RequestAttachmentState::Prepared
                    | RequestAttachmentState::Uploaded
                    | RequestAttachmentState::Rejected
            )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentBinding {
    pub attachment_id: String,
    pub request_id: String,
    pub target: RequestAttachmentBindingTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentPartReceipt {
    pub part_number: u32,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentStagedPart {
    pub receipt: RequestAttachmentPartReceipt,
    pub object_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestMediaChunk {
    pub part_number: u32,
    pub plaintext_offset: u64,
    pub plaintext_bytes: u64,
    pub sha256: String,
    pub object_key: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestMediaObject {
    pub media_type: String,
    pub plaintext_bytes: u64,
    pub sha256: String,
    pub chunks: Vec<RequestMediaChunk>,
}

impl RequestMediaObject {
    pub fn from_parts(
        media_type: impl Into<String>,
        sha256: impl Into<String>,
        mut parts: Vec<RequestAttachmentStagedPart>,
    ) -> Self {
        parts.sort_by_key(|part| part.receipt.part_number);
        let mut plaintext_bytes = 0_u64;
        let chunks = parts
            .into_iter()
            .map(|part| {
                let chunk = RequestMediaChunk {
                    part_number: part.receipt.part_number,
                    plaintext_offset: plaintext_bytes,
                    plaintext_bytes: part.receipt.size_bytes,
                    sha256: part.receipt.sha256,
                    object_key: part.object_key,
                };
                plaintext_bytes = plaintext_bytes.saturating_add(chunk.plaintext_bytes);
                chunk
            })
            .collect();
        Self {
            media_type: media_type.into(),
            plaintext_bytes,
            sha256: sha256.into(),
            chunks,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAttachmentSource {
    pub detected_media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub image: Option<RequestAttachmentImageMetadata>,
    pub video: Option<RequestAttachmentVideoMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAttachmentProcessingLease {
    pub attachment_id: String,
    pub kind: RequestAttachmentKind,
    pub lease_token: String,
    pub lease_generation: u64,
    pub attempt: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestAttachmentCleanupLease {
    pub attachment_id: String,
    pub object_keys: Vec<String>,
    pub lease_token: String,
    pub lease_generation: u64,
    pub attempt: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequestAttachmentLimits {
    pub max_photo_bytes: u64,
    pub max_video_bytes: u64,
    pub max_video_duration_seconds: u64,
    pub max_attachments_per_content: usize,
    pub max_request_source_bytes: u64,
    pub max_repository_storage_bytes: u64,
    pub max_photo_pixels: u64,
    pub preferred_part_bytes: u64,
    pub max_concurrent_parts: usize,
    pub incomplete_upload_ttl_seconds: u64,
    pub unbound_attachment_ttl_seconds: u64,
}

impl Default for RequestAttachmentLimits {
    fn default() -> Self {
        Self {
            max_photo_bytes: 25 * 1024 * 1024,
            max_video_bytes: 500 * 1024 * 1024,
            max_video_duration_seconds: 10 * 60,
            max_attachments_per_content: 10,
            max_request_source_bytes: 2 * 1024 * 1024 * 1024,
            max_repository_storage_bytes: 20 * 1024 * 1024 * 1024,
            max_photo_pixels: 50_000_000,
            preferred_part_bytes: REQUEST_MEDIA_CHUNK_BYTES,
            max_concurrent_parts: 2,
            incomplete_upload_ttl_seconds: 24 * 60 * 60,
            unbound_attachment_ttl_seconds: 7 * 24 * 60 * 60,
        }
    }
}
