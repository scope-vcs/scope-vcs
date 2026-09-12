mod access;
mod bindings;
mod cleanup;
mod locks;
mod persistence;
mod processing;
mod processing_support;
mod upload;

use super::MediaStore;
pub(in crate::db) use bindings::replace_bindings_for_markdown;
pub(in crate::db) use cleanup::{tombstone_repository_attachments, tombstone_request_attachments};

#[cfg(test)]
mod tests;

use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentDerivative, RequestAttachmentFailure,
    RequestAttachmentPartReceipt, RequestAttachmentTarget,
};

#[derive(Clone, Debug)]
pub struct PrepareRequestAttachmentCommand {
    pub attachment_id: String,
    pub upload_id: String,
    pub operation_id: String,
    pub request_id: String,
    pub actor_user_id: String,
    pub target: RequestAttachmentTarget,
    pub filename: String,
    pub declared_media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct PreparedRequestAttachment {
    pub attachment: RequestAttachment,
    pub upload_id: String,
    pub acknowledged_parts: Vec<RequestAttachmentPartReceipt>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRequestAttachmentPart {
    pub receipt: RequestAttachmentPartReceipt,
    pub object_key: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorePartResult {
    Recorded,
    AlreadyRecorded(StoredRequestAttachmentPart),
    WriteLeaseLost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReserveUploadPartResult {
    Write(StoredRequestAttachmentPart),
    Stored(StoredRequestAttachmentPart),
    Busy,
}

#[derive(Clone, Debug)]
pub struct FinishRequestAttachmentUploadCommand {
    pub request_id: String,
    pub attachment_id: String,
    pub upload_id: String,
    pub actor_user_id: String,
    pub parts: Vec<RequestAttachmentPartReceipt>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct AuthorizedRequestAttachment {
    pub attachment: RequestAttachment,
    pub repository_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestMediaObjectTarget<'a> {
    Original,
    Derivative(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestMediaManifest {
    pub id: String,
    pub attachment_id: String,
    pub derivative_id: Option<String>,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub chunks: Vec<RequestMediaChunk>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestMediaChunk {
    pub index: u32,
    pub object_key: String,
    pub plaintext_offset: u64,
    pub plaintext_size_bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRequestMediaManifest {
    pub id: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub chunks: Vec<RequestMediaChunk>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedRequestAttachmentSource {
    pub detected_media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub duration_millis: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRequestAttachmentDerivative {
    pub derivative: RequestAttachmentDerivative,
    pub manifest: CompletedRequestMediaManifest,
}

#[derive(Clone, Debug)]
pub struct ValidateRequestAttachmentSourceCommand {
    pub attachment_id: String,
    pub lease_token: String,
    pub lease_generation: u64,
    pub source: ValidatedRequestAttachmentSource,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CompleteRequestAttachmentProcessingCommand {
    pub attachment_id: String,
    pub lease_token: String,
    pub lease_generation: u64,
    pub source: ValidatedRequestAttachmentSource,
    pub derivatives: Vec<CompletedRequestAttachmentDerivative>,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct FailRequestAttachmentProcessingCommand {
    pub attachment_id: String,
    pub lease_token: String,
    pub lease_generation: u64,
    pub failure: RequestAttachmentFailure,
    pub now_unix: u64,
    pub retry_at_unix: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MediaLeaseMutation<T> {
    Applied(T),
    LeaseLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestAttachmentCleanupReason {
    IncompleteUploadExpired,
    UnboundDraftExpired,
    RequestDeleted,
    RepositoryDeleted,
}

impl RequestAttachmentCleanupReason {
    fn as_str(self) -> &'static str {
        match self {
            Self::IncompleteUploadExpired => "IncompleteUploadExpired",
            Self::UnboundDraftExpired => "UnboundDraftExpired",
            Self::RequestDeleted => "RequestDeleted",
            Self::RepositoryDeleted => "RepositoryDeleted",
        }
    }
}
