mod access;
mod bindings;
mod budget;
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
    RequestAttachmentPartReceipt, RequestAttachmentSource, RequestAttachmentStagedPart,
    RequestAttachmentTarget, RequestMediaChunk,
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
pub enum StorePartResult {
    Recorded,
    AlreadyRecorded(RequestAttachmentStagedPart),
    WriteLeaseLost,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReserveUploadPartResult {
    Write(RequestAttachmentStagedPart),
    Stored(RequestAttachmentStagedPart),
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestMediaObjectTarget<'a> {
    Original,
    Derivative(&'a str),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedRequestAttachmentDerivative {
    pub derivative: RequestAttachmentDerivative,
    pub chunks: Vec<RequestMediaChunk>,
}

#[derive(Clone, Debug)]
pub struct ValidateRequestAttachmentSourceCommand {
    pub attachment_id: String,
    pub lease_token: String,
    pub lease_generation: u64,
    pub source: RequestAttachmentSource,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct CompleteRequestAttachmentProcessingCommand {
    pub attachment_id: String,
    pub lease_token: String,
    pub lease_generation: u64,
    pub source: RequestAttachmentSource,
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
