use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentTargetKind {
    Description,
    Discussion,
    Reply,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentTargetInput {
    pub kind: RequestAttachmentTargetKind,
    pub discussion_id: Option<String>,
}

impl TryFrom<RequestAttachmentTargetInput>
    for scope_domain::requests::attachments::RequestAttachmentTarget
{
    type Error = scope_domain::error::DomainError;

    fn try_from(value: RequestAttachmentTargetInput) -> Result<Self, Self::Error> {
        use scope_domain::requests::attachments::RequestAttachmentTarget;
        let discussion_id = value
            .discussion_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        match value.kind {
            RequestAttachmentTargetKind::Description if discussion_id.is_none() => {
                Ok(RequestAttachmentTarget::Description)
            }
            RequestAttachmentTargetKind::Description => {
                Err(scope_domain::error::DomainError::invalid_input(
                    "description attachment target cannot include a discussion id",
                ))
            }
            RequestAttachmentTargetKind::Discussion => {
                Ok(RequestAttachmentTarget::Discussion { discussion_id })
            }
            RequestAttachmentTargetKind::Reply => discussion_id
                .map(|discussion_id| RequestAttachmentTarget::Reply { discussion_id })
                .ok_or_else(|| {
                    scope_domain::error::DomainError::invalid_input(
                        "reply attachment target requires a discussion id",
                    )
                }),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentKind {
    Photo,
    Video,
}

impl From<scope_domain::requests::attachments::RequestAttachmentKind> for RequestAttachmentKind {
    fn from(value: scope_domain::requests::attachments::RequestAttachmentKind) -> Self {
        match value {
            scope_domain::requests::attachments::RequestAttachmentKind::Photo => Self::Photo,
            scope_domain::requests::attachments::RequestAttachmentKind::Video => Self::Video,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentState {
    Prepared,
    Uploaded,
    Processing,
    Ready,
    Failed,
    Rejected,
}

impl From<scope_domain::requests::attachments::RequestAttachmentState> for RequestAttachmentState {
    fn from(value: scope_domain::requests::attachments::RequestAttachmentState) -> Self {
        use scope_domain::requests::attachments::RequestAttachmentState as Domain;
        match value {
            Domain::Prepared => Self::Prepared,
            Domain::Uploaded => Self::Uploaded,
            Domain::Processing => Self::Processing,
            Domain::Ready => Self::Ready,
            Domain::Failed => Self::Failed,
            Domain::Rejected => Self::Rejected,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentDerivativeKind {
    ImagePreview,
    VideoPlayback,
    VideoPoster,
}

impl From<scope_domain::requests::attachments::RequestAttachmentDerivativeKind>
    for RequestAttachmentDerivativeKind
{
    fn from(value: scope_domain::requests::attachments::RequestAttachmentDerivativeKind) -> Self {
        use scope_domain::requests::attachments::RequestAttachmentDerivativeKind as Domain;
        match value {
            Domain::ImagePreview => Self::ImagePreview,
            Domain::VideoPlayback => Self::VideoPlayback,
            Domain::VideoPoster => Self::VideoPoster,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentFailureCode {
    InvalidMedia,
    CorruptMedia,
    UnsupportedMedia,
    MediaLimitExceeded,
    CodecFailed,
    StorageUnavailable,
    Internal,
}

impl From<scope_domain::requests::attachments::RequestAttachmentFailureCode>
    for RequestAttachmentFailureCode
{
    fn from(value: scope_domain::requests::attachments::RequestAttachmentFailureCode) -> Self {
        use scope_domain::requests::attachments::RequestAttachmentFailureCode as Domain;
        match value {
            Domain::InvalidMedia => Self::InvalidMedia,
            Domain::CorruptMedia => Self::CorruptMedia,
            Domain::UnsupportedMedia => Self::UnsupportedMedia,
            Domain::MediaLimitExceeded => Self::MediaLimitExceeded,
            Domain::CodecFailed => Self::CodecFailed,
            Domain::StorageUnavailable => Self::StorageUnavailable,
            Domain::Internal => Self::Internal,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentLimitsResponse {
    pub accepted_photo_media_types: Vec<String>,
    pub accepted_video_media_types: Vec<String>,
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

impl From<scope_domain::requests::attachments::RequestAttachmentLimits>
    for RequestAttachmentLimitsResponse
{
    fn from(value: scope_domain::requests::attachments::RequestAttachmentLimits) -> Self {
        Self {
            accepted_photo_media_types:
                scope_domain::requests::attachments::REQUEST_ATTACHMENT_PHOTO_MEDIA_TYPES
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            accepted_video_media_types:
                scope_domain::requests::attachments::REQUEST_ATTACHMENT_VIDEO_MEDIA_TYPES
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
            max_photo_bytes: value.max_photo_bytes,
            max_video_bytes: value.max_video_bytes,
            max_video_duration_seconds: value.max_video_duration_seconds,
            max_attachments_per_content: value.max_attachments_per_content,
            max_request_source_bytes: value.max_request_source_bytes,
            max_repository_storage_bytes: value.max_repository_storage_bytes,
            max_photo_pixels: value.max_photo_pixels,
            preferred_part_bytes: value.preferred_part_bytes,
            max_concurrent_parts: value.max_concurrent_parts,
            incomplete_upload_ttl_seconds: value.incomplete_upload_ttl_seconds,
            unbound_attachment_ttl_seconds: value.unbound_attachment_ttl_seconds,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PrepareRequestAttachmentRequest {
    pub operation_id: String,
    pub target: RequestAttachmentTargetInput,
    pub filename: String,
    pub declared_media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct PrepareRequestAttachmentResponse {
    pub attachment: RequestAttachmentResponse,
    pub transfer: RequestAttachmentTransferResponse,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentTransferResponse {
    pub upload_id: String,
    pub media_base_url: String,
    pub grant: String,
    pub expires_at_unix: u64,
    pub preferred_part_bytes: u64,
    pub max_concurrent_parts: usize,
    pub acknowledged_parts: Vec<RequestAttachmentPartReceiptResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentPartReceiptResponse {
    pub part_number: u32,
    pub size_bytes: u64,
    pub sha256: String,
}

impl From<scope_domain::requests::attachments::RequestAttachmentPartReceipt>
    for RequestAttachmentPartReceiptResponse
{
    fn from(value: scope_domain::requests::attachments::RequestAttachmentPartReceipt) -> Self {
        Self {
            part_number: value.part_number,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}

impl From<RequestAttachmentPartReceiptResponse>
    for scope_domain::requests::attachments::RequestAttachmentPartReceipt
{
    fn from(value: RequestAttachmentPartReceiptResponse) -> Self {
        Self {
            part_number: value.part_number,
            size_bytes: value.size_bytes,
            sha256: value.sha256,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct FinishRequestAttachmentRequest {
    pub upload_id: String,
    pub parts: Vec<RequestAttachmentPartReceiptResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RetryRequestAttachmentRequest {
    pub operation_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentListResponse {
    pub attachments: Vec<RequestAttachmentResponse>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentResponse {
    pub id: String,
    pub request_id: String,
    pub uploader_user_id: String,
    pub filename: String,
    pub declared_media_type: String,
    pub detected_media_type: Option<String>,
    pub kind: RequestAttachmentKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub state: RequestAttachmentState,
    pub original_download_available: bool,
    pub failure: Option<RequestAttachmentFailureResponse>,
    pub image: Option<RequestAttachmentImageMetadataResponse>,
    pub video: Option<RequestAttachmentVideoMetadataResponse>,
    pub derivatives: Vec<RequestAttachmentDerivativeResponse>,
    pub created_at_unix: u64,
    pub updated_at_unix: u64,
}

impl From<scope_domain::requests::attachments::RequestAttachment> for RequestAttachmentResponse {
    fn from(value: scope_domain::requests::attachments::RequestAttachment) -> Self {
        Self::from(&value)
    }
}

impl From<&scope_domain::requests::attachments::RequestAttachment> for RequestAttachmentResponse {
    fn from(value: &scope_domain::requests::attachments::RequestAttachment) -> Self {
        Self {
            id: value.id.clone(),
            request_id: value.request_id.clone(),
            uploader_user_id: value.uploader_user_id.clone(),
            filename: value.filename.clone(),
            declared_media_type: value.declared_media_type.clone(),
            detected_media_type: value.detected_media_type.clone(),
            kind: value.kind.into(),
            size_bytes: value.size_bytes,
            sha256: value.sha256.clone(),
            state: value.state.into(),
            original_download_available: value.original_is_grantable(),
            failure: value.failure.as_ref().map(Into::into),
            image: value.image.as_ref().map(Into::into),
            video: value.video.as_ref().map(Into::into),
            derivatives: value.derivatives.iter().map(Into::into).collect(),
            created_at_unix: value.created_at_unix,
            updated_at_unix: value.updated_at_unix,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentImageMetadataResponse {
    pub width: u32,
    pub height: u32,
}

impl From<&scope_domain::requests::attachments::RequestAttachmentImageMetadata>
    for RequestAttachmentImageMetadataResponse
{
    fn from(value: &scope_domain::requests::attachments::RequestAttachmentImageMetadata) -> Self {
        Self {
            width: value.width,
            height: value.height,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentVideoMetadataResponse {
    pub width: u32,
    pub height: u32,
    pub duration_millis: u64,
}

impl From<&scope_domain::requests::attachments::RequestAttachmentVideoMetadata>
    for RequestAttachmentVideoMetadataResponse
{
    fn from(value: &scope_domain::requests::attachments::RequestAttachmentVideoMetadata) -> Self {
        Self {
            width: value.width,
            height: value.height,
            duration_millis: value.duration_millis,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentDerivativeResponse {
    pub id: String,
    pub kind: RequestAttachmentDerivativeKind,
    pub media_type: String,
    pub size_bytes: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_millis: Option<u64>,
}

impl From<&scope_domain::requests::attachments::RequestAttachmentDerivative>
    for RequestAttachmentDerivativeResponse
{
    fn from(value: &scope_domain::requests::attachments::RequestAttachmentDerivative) -> Self {
        Self {
            id: value.id.clone(),
            kind: value.kind.into(),
            media_type: value.media_type.clone(),
            size_bytes: value.object.size_bytes,
            width: value.width,
            height: value.height,
            duration_millis: value.duration_millis,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentFailureResponse {
    pub code: RequestAttachmentFailureCode,
    pub message: String,
    pub retryable: bool,
}

impl From<&scope_domain::requests::attachments::RequestAttachmentFailure>
    for RequestAttachmentFailureResponse
{
    fn from(value: &scope_domain::requests::attachments::RequestAttachmentFailure) -> Self {
        Self {
            code: value.code.into(),
            message: value.message.clone(),
            retryable: value.retryable,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(rename_all = "snake_case"))]
pub enum RequestAttachmentMediaTarget {
    Original,
    Derivative { derivative_id: String },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRequestAttachmentMediaGrantRequest {
    pub target: RequestAttachmentMediaTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CreateRequestAttachmentMediaGrantResponse {
    pub media_url: String,
    pub grant: String,
    pub expires_at_unix: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub enum RequestAttachmentMediaGrantMethod {
    Get,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentMediaGrantClaims {
    pub attachment_id: String,
    pub repository_id: String,
    pub request_id: String,
    pub viewer_user_id: Option<String>,
    pub method: RequestAttachmentMediaGrantMethod,
    pub target: RequestAttachmentMediaTarget,
    pub expires_at_unix: u64,
}

impl RequestAttachmentMediaGrantClaims {
    pub fn allows(
        &self,
        attachment_id: &str,
        target: &RequestAttachmentMediaTarget,
        now_unix: u64,
    ) -> bool {
        now_unix < self.expires_at_unix
            && self.method == RequestAttachmentMediaGrantMethod::Get
            && self.attachment_id == attachment_id
            && &self.target == target
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct RequestAttachmentUploadGrantClaims {
    pub attachment_id: String,
    pub repository_id: String,
    pub request_id: String,
    pub uploader_user_id: String,
    pub upload_id: String,
    pub expires_at_unix: u64,
}

impl RequestAttachmentUploadGrantClaims {
    pub fn allows_part(&self, upload_id: &str, attachment_id: &str, now_unix: u64) -> bool {
        now_unix < self.expires_at_unix
            && self.upload_id == upload_id
            && self.attachment_id == attachment_id
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_enforce_discussion_shape() {
        assert!(
            scope_domain::requests::attachments::RequestAttachmentTarget::try_from(
                RequestAttachmentTargetInput {
                    kind: RequestAttachmentTargetKind::Reply,
                    discussion_id: None,
                }
            )
            .is_err()
        );
        assert!(
            scope_domain::requests::attachments::RequestAttachmentTarget::try_from(
                RequestAttachmentTargetInput {
                    kind: RequestAttachmentTargetKind::Description,
                    discussion_id: Some("disc_1".into()),
                }
            )
            .is_err()
        );
    }

    #[test]
    fn media_claims_are_exact_and_expire_exclusively() {
        let target = RequestAttachmentMediaTarget::Original;
        let claims = RequestAttachmentMediaGrantClaims {
            attachment_id: "att_1".into(),
            repository_id: "repo_1".into(),
            request_id: "req_1".into(),
            viewer_user_id: None,
            method: RequestAttachmentMediaGrantMethod::Get,
            target: target.clone(),
            expires_at_unix: 20,
        };
        assert!(claims.allows("att_1", &target, 19));
        assert!(!claims.allows("att_1", &target, 20));
        assert!(!claims.allows(
            "att_1",
            &RequestAttachmentMediaTarget::Derivative {
                derivative_id: "preview".into()
            },
            19
        ));
    }
}
