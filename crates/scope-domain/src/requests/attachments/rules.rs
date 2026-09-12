use super::{
    RequestAttachment, RequestAttachmentBinding, RequestAttachmentBindingTarget,
    RequestAttachmentDerivative, RequestAttachmentDerivativeKind, RequestAttachmentFailure,
    RequestAttachmentFailureCode, RequestAttachmentImageMetadata, RequestAttachmentKind,
    RequestAttachmentLimits, RequestAttachmentPartReceipt, RequestAttachmentProcessingLease,
    RequestAttachmentState, RequestAttachmentStoredObject, RequestAttachmentTarget,
    RequestAttachmentVideoMetadata,
    references::{
        REQUEST_ATTACHMENT_ID_MAX_BYTES, request_attachment_references, validate_attachment_id,
    },
};
use crate::{error::DomainError, requests::validate_required};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct PrepareRequestAttachmentInput {
    pub attachment_id: String,
    pub repository_id: String,
    pub request_id: String,
    pub uploader_user_id: String,
    pub upload_id: String,
    pub operation_id: String,
    pub target: RequestAttachmentTarget,
    pub filename: String,
    pub declared_media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub actor_can_write_target: bool,
    pub request_is_open: bool,
    pub target_attachment_count: usize,
    pub request_source_bytes: u64,
    /// Source and derivative bytes already reserved or stored in the repository.
    pub repository_reserved_bytes: u64,
    pub now_unix: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrepareRequestAttachmentDecision {
    pub attachment: RequestAttachment,
    pub reserved_source_bytes: u64,
    pub reserved_derivative_bytes: u64,
}

pub fn validate_prepare_attachment(
    input: PrepareRequestAttachmentInput,
) -> Result<PrepareRequestAttachmentDecision, DomainError> {
    let limits = RequestAttachmentLimits::default();
    validate_attachment_id(&input.attachment_id)?;
    for (label, value) in [
        ("repository id", input.repository_id.as_str()),
        ("request id", input.request_id.as_str()),
        ("uploader user id", input.uploader_user_id.as_str()),
        ("upload id", input.upload_id.as_str()),
        ("operation id", input.operation_id.as_str()),
    ] {
        validate_required(label, value)?;
    }
    validate_identifier("upload id", &input.upload_id)?;
    validate_identifier("operation id", &input.operation_id)?;
    validate_filename(&input.filename)?;
    validate_target(&input.target)?;
    if !input.actor_can_write_target {
        return Err(DomainError::forbidden(
            "request attachment target write access required",
        ));
    }
    if !input.request_is_open {
        return Err(DomainError::conflict(
            "request attachments require a writable request",
        ));
    }
    if input.target_attachment_count >= limits.max_attachments_per_content {
        return Err(DomainError::invalid_input(format!(
            "request content supports at most {} attachments",
            limits.max_attachments_per_content
        )));
    }
    if input.size_bytes == 0 {
        return Err(DomainError::invalid_input(
            "request attachment cannot be empty",
        ));
    }
    let kind =
        RequestAttachmentKind::from_media_type(&input.declared_media_type).ok_or_else(|| {
            DomainError::invalid_input("request attachment media type is unsupported")
        })?;
    let max_bytes = match kind {
        RequestAttachmentKind::Photo => limits.max_photo_bytes,
        RequestAttachmentKind::Video => limits.max_video_bytes,
    };
    if input.size_bytes > max_bytes {
        return Err(DomainError::invalid_input(format!(
            "request attachment exceeds the {max_bytes} byte {kind:?} limit"
        )));
    }
    validate_sha256(&input.sha256)?;
    let request_total = checked_usage(input.request_source_bytes, input.size_bytes)?;
    if request_total > limits.max_request_source_bytes {
        return Err(DomainError::invalid_input(
            "request attachment source byte budget exceeded",
        ));
    }
    let reserved_derivative_bytes = reserved_attachment_derivative_bytes(kind, input.size_bytes)?;
    let new_reservation = checked_usage(input.size_bytes, reserved_derivative_bytes)?;
    let repository_total = checked_usage(input.repository_reserved_bytes, new_reservation)?;
    if repository_total > limits.max_repository_storage_bytes {
        return Err(DomainError::invalid_input(
            "repository attachment storage byte budget exceeded",
        ));
    }
    let upload_expires_at_unix = input
        .now_unix
        .checked_add(limits.incomplete_upload_ttl_seconds)
        .ok_or_else(|| DomainError::invariant_violation("upload expiry overflow"))?;
    let attachment = RequestAttachment {
        id: input.attachment_id,
        repository_id: input.repository_id,
        request_id: input.request_id,
        uploader_user_id: input.uploader_user_id,
        upload_id: input.upload_id,
        operation_id: input.operation_id,
        target: input.target,
        filename: input.filename,
        declared_media_type: input.declared_media_type.to_ascii_lowercase(),
        detected_media_type: None,
        kind,
        size_bytes: input.size_bytes,
        sha256: input.sha256.to_ascii_lowercase(),
        state: RequestAttachmentState::Prepared,
        original: None,
        original_validated_at_unix: None,
        failure: None,
        image: None,
        video: None,
        derivatives: Vec::new(),
        created_at_unix: input.now_unix,
        updated_at_unix: input.now_unix,
        upload_expires_at_unix,
    };
    Ok(PrepareRequestAttachmentDecision {
        attachment,
        reserved_source_bytes: input.size_bytes,
        reserved_derivative_bytes,
    })
}

fn reserved_attachment_derivative_bytes(
    kind: RequestAttachmentKind,
    source_size_bytes: u64,
) -> Result<u64, DomainError> {
    source_size_bytes
        .checked_mul(match kind {
            RequestAttachmentKind::Photo => 1,
            RequestAttachmentKind::Video => 2,
        })
        .ok_or_else(|| DomainError::invalid_input("request attachment reservation overflow"))
}

pub fn finish_attachment_upload(
    attachment: &RequestAttachment,
    actor_user_id: &str,
    upload_id: &str,
    receipts: &[RequestAttachmentPartReceipt],
    original: RequestAttachmentStoredObject,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    if attachment.uploader_user_id != actor_user_id {
        return Err(DomainError::forbidden(
            "only the attachment uploader may finish an upload",
        ));
    }
    if attachment.upload_id != upload_id {
        return Err(DomainError::conflict(
            "request attachment upload id mismatch",
        ));
    }
    validate_part_receipts(attachment, receipts)?;
    if original.size_bytes != attachment.size_bytes || original.sha256 != attachment.sha256 {
        return Err(DomainError::conflict(
            "request attachment original does not match the prepared file",
        ));
    }
    validate_required(
        "request attachment original object key",
        &original.object_key,
    )?;
    if attachment.state != RequestAttachmentState::Prepared {
        if attachment.original.as_ref() == Some(&original) {
            return Ok(attachment.clone());
        }
        return Err(DomainError::conflict(
            "request attachment upload is already finalized with another original",
        ));
    }
    if now_unix >= attachment.upload_expires_at_unix {
        return Err(DomainError::conflict("request attachment upload expired"));
    }
    transition_attachment(attachment, RequestAttachmentState::Uploaded, None, now_unix).map(
        |mut attachment| {
            attachment.original = Some(original);
            attachment
        },
    )
}

pub fn validate_attachment_part(
    attachment: &RequestAttachment,
    receipt: &RequestAttachmentPartReceipt,
) -> Result<(), DomainError> {
    if attachment.state != RequestAttachmentState::Prepared {
        return Err(DomainError::conflict(
            "request attachment upload is already finalized",
        ));
    }
    validate_part_range(attachment.size_bytes, receipt)
}

fn validate_part_range(
    attachment_size_bytes: u64,
    receipt: &RequestAttachmentPartReceipt,
) -> Result<(), DomainError> {
    let limits = RequestAttachmentLimits::default();
    if receipt.part_number == 0 || limits.preferred_part_bytes == 0 {
        return Err(DomainError::invalid_input(
            "request attachment part number and preferred size must be positive",
        ));
    }
    let offset = u64::from(receipt.part_number - 1)
        .checked_mul(limits.preferred_part_bytes)
        .ok_or_else(|| DomainError::invalid_input("request attachment part offset overflow"))?;
    let remaining = attachment_size_bytes.checked_sub(offset).ok_or_else(|| {
        DomainError::invalid_input("request attachment part starts beyond the prepared file")
    })?;
    if remaining == 0 || receipt.size_bytes != remaining.min(limits.preferred_part_bytes) {
        return Err(DomainError::invalid_input(
            "request attachment part size does not match its prepared offset",
        ));
    }
    validate_sha256(&receipt.sha256)
}

pub fn transition_attachment(
    attachment: &RequestAttachment,
    target: RequestAttachmentState,
    failure: Option<RequestAttachmentFailure>,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    if attachment.state == target {
        return Ok(attachment.clone());
    }
    let allowed = matches!(
        (attachment.state, target),
        (
            RequestAttachmentState::Prepared,
            RequestAttachmentState::Uploaded
        ) | (
            RequestAttachmentState::Uploaded,
            RequestAttachmentState::Processing
                | RequestAttachmentState::Ready
                | RequestAttachmentState::Failed
                | RequestAttachmentState::Rejected
        ) | (
            RequestAttachmentState::Processing,
            RequestAttachmentState::Ready
                | RequestAttachmentState::Failed
                | RequestAttachmentState::Rejected
        ) | (
            RequestAttachmentState::Failed,
            RequestAttachmentState::Processing
        )
    );
    if !allowed {
        return Err(DomainError::conflict(format!(
            "request attachment cannot transition from {:?} to {target:?}",
            attachment.state
        )));
    }
    validate_failure_for_state(target, failure.as_ref())?;
    if attachment.state == RequestAttachmentState::Failed
        && target == RequestAttachmentState::Processing
        && !attachment
            .failure
            .as_ref()
            .is_some_and(|failure| failure.retryable)
    {
        return Err(DomainError::conflict(
            "request attachment failure is not retryable",
        ));
    }
    let mut next = attachment.clone();
    next.state = target;
    next.failure = failure;
    next.updated_at_unix = now_unix;
    Ok(next)
}

#[allow(clippy::too_many_arguments)]
pub fn validate_processing_completion(
    attachment: &RequestAttachment,
    lease: &RequestAttachmentProcessingLease,
    lease_token: &str,
    lease_generation: u64,
    detected_media_type: String,
    image: Option<RequestAttachmentImageMetadata>,
    video: Option<RequestAttachmentVideoMetadata>,
    derivatives: Vec<RequestAttachmentDerivative>,
    validated_at_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    let mut next = mark_processing_source_validated(
        attachment,
        lease,
        lease_token,
        lease_generation,
        detected_media_type,
        image,
        video,
        validated_at_unix,
    )?;
    validate_derivatives(next.kind, &derivatives)?;
    next = transition_attachment(
        &next,
        RequestAttachmentState::Ready,
        None,
        validated_at_unix,
    )?;
    next.derivatives = derivatives;
    Ok(next)
}

#[allow(clippy::too_many_arguments)]
pub fn mark_processing_source_validated(
    attachment: &RequestAttachment,
    lease: &RequestAttachmentProcessingLease,
    lease_token: &str,
    lease_generation: u64,
    detected_media_type: String,
    image: Option<RequestAttachmentImageMetadata>,
    video: Option<RequestAttachmentVideoMetadata>,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    validate_processing_lease(attachment, lease, lease_token, lease_generation, now_unix)?;
    validate_attachment_source(
        attachment,
        &detected_media_type,
        image.as_ref(),
        video.as_ref(),
    )?;
    let mut next = transition_attachment(
        attachment,
        RequestAttachmentState::Processing,
        None,
        now_unix,
    )?;
    next.detected_media_type = Some(detected_media_type.trim().to_ascii_lowercase());
    next.original_validated_at_unix = Some(now_unix);
    next.image = image;
    next.video = video;
    Ok(next)
}

pub fn validate_processing_failure(
    attachment: &RequestAttachment,
    lease: &RequestAttachmentProcessingLease,
    lease_token: &str,
    lease_generation: u64,
    failure: RequestAttachmentFailure,
    original_validated: bool,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    validate_processing_lease(attachment, lease, lease_token, lease_generation, now_unix)?;
    let target = if failure.retryable {
        RequestAttachmentState::Failed
    } else {
        RequestAttachmentState::Rejected
    };
    let mut next = transition_attachment(attachment, target, Some(failure), now_unix)?;
    next.original_validated_at_unix = attachment
        .original_validated_at_unix
        .or_else(|| original_validated.then_some(now_unix));
    Ok(next)
}

pub fn retry_attachment_processing(
    attachment: &RequestAttachment,
    actor_can_write_target: bool,
    now_unix: u64,
) -> Result<RequestAttachment, DomainError> {
    if !actor_can_write_target {
        return Err(DomainError::forbidden(
            "request attachment target write access required",
        ));
    }
    transition_attachment(
        attachment,
        RequestAttachmentState::Processing,
        None,
        now_unix,
    )
}

pub fn replace_attachment_bindings(
    request_id: &str,
    actor_user_id: &str,
    actor_can_write_target: bool,
    target: RequestAttachmentBindingTarget,
    markdown: &str,
    attachments: &[RequestAttachment],
    existing_bindings: &[RequestAttachmentBinding],
) -> Result<Vec<RequestAttachmentBinding>, DomainError> {
    let limits = RequestAttachmentLimits::default();
    if !actor_can_write_target {
        return Err(DomainError::forbidden(
            "request attachment target write access required",
        ));
    }
    validate_binding_target(&target)?;
    let references = request_attachment_references(markdown)?;
    if references.len() > limits.max_attachments_per_content {
        return Err(DomainError::invalid_input(format!(
            "request content supports at most {} attachments",
            limits.max_attachments_per_content
        )));
    }
    let by_id: BTreeMap<_, _> = attachments
        .iter()
        .map(|attachment| (attachment.id.as_str(), attachment))
        .collect();
    let mut projected = Vec::with_capacity(references.len());
    for attachment_id in references {
        let attachment = by_id.get(attachment_id.as_str()).ok_or_else(|| {
            DomainError::not_found("request attachment reference was not prepared")
        })?;
        if attachment.request_id != request_id {
            return Err(DomainError::forbidden(
                "request attachment belongs to another request",
            ));
        }
        let current_binding = existing_bindings
            .iter()
            .find(|binding| binding.attachment_id == attachment_id);
        if current_binding.is_some_and(|binding| binding.target != target) {
            return Err(DomainError::conflict(
                "request attachment is already bound to other content",
            ));
        }
        let preserving_current = current_binding.is_some_and(|binding| binding.target == target);
        if attachment.uploader_user_id != actor_user_id && !preserving_current {
            return Err(DomainError::forbidden(
                "only the uploader may publish a draft request attachment",
            ));
        }
        if attachment.state == RequestAttachmentState::Prepared {
            return Err(DomainError::conflict(
                "request attachment upload must finish before publishing",
            ));
        }
        validate_target_matches_binding(&attachment.target, &target)?;
        projected.push(RequestAttachmentBinding {
            attachment_id,
            request_id: request_id.to_string(),
            target: target.clone(),
        });
    }
    Ok(projected)
}

pub fn can_view_request_attachment(
    attachment: &RequestAttachment,
    bindings: &[RequestAttachmentBinding],
    viewer_user_id: Option<&str>,
    description_visible: bool,
    discussion_visible: bool,
) -> bool {
    if viewer_user_id == Some(attachment.uploader_user_id.as_str()) {
        return true;
    }
    bindings
        .iter()
        .filter(|binding| binding.attachment_id == attachment.id)
        .any(|binding| match binding.target {
            RequestAttachmentBindingTarget::Description => description_visible,
            RequestAttachmentBindingTarget::Discussion { .. }
            | RequestAttachmentBindingTarget::Reply { .. } => discussion_visible,
        })
}

/// A newly granted lease needs a token and an expiry after the grant time.
pub fn validate_lease_grant(
    lease_token: &str,
    now_unix: u64,
    lease_expires_at_unix: u64,
) -> Result<(), DomainError> {
    if lease_token.trim().is_empty() || lease_expires_at_unix <= now_unix {
        return Err(DomainError::invalid_input(
            "lease must have a token and a future expiry",
        ));
    }
    Ok(())
}

pub fn validate_cleanup_lease(
    attachment_id: &str,
    lease: &super::RequestAttachmentCleanupLease,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
) -> Result<(), DomainError> {
    if lease.attachment_id != attachment_id
        || lease.lease_token != lease_token
        || lease.lease_generation != lease_generation
        || now_unix >= lease.lease_expires_at_unix
    {
        return Err(DomainError::conflict(
            "stale request attachment cleanup lease",
        ));
    }
    Ok(())
}

fn validate_processing_lease(
    attachment: &RequestAttachment,
    lease: &RequestAttachmentProcessingLease,
    lease_token: &str,
    lease_generation: u64,
    now_unix: u64,
) -> Result<(), DomainError> {
    if lease.attachment_id != attachment.id
        || lease.repository_id != attachment.repository_id
        || lease.request_id != attachment.request_id
        || lease.lease_token != lease_token
        || lease.lease_generation != lease_generation
        || now_unix >= lease.lease_expires_at_unix
    {
        return Err(DomainError::conflict(
            "stale request attachment processing lease",
        ));
    }
    if !matches!(
        attachment.state,
        RequestAttachmentState::Uploaded | RequestAttachmentState::Processing
    ) {
        return Err(DomainError::conflict(
            "request attachment is not processable",
        ));
    }
    Ok(())
}

fn validate_attachment_source(
    attachment: &RequestAttachment,
    detected_media_type: &str,
    image: Option<&RequestAttachmentImageMetadata>,
    video: Option<&RequestAttachmentVideoMetadata>,
) -> Result<(), DomainError> {
    let limits = RequestAttachmentLimits::default();
    let detected_kind =
        RequestAttachmentKind::from_media_type(detected_media_type).ok_or_else(|| {
            DomainError::invalid_input("detected attachment media type is unsupported")
        })?;
    if detected_kind != attachment.kind {
        return Err(DomainError::invalid_input(
            "detected attachment kind does not match the prepared media type",
        ));
    }
    let original = attachment
        .original
        .as_ref()
        .ok_or_else(|| DomainError::conflict("request attachment original is not sealed"))?;
    if original.size_bytes != attachment.size_bytes || original.sha256 != attachment.sha256 {
        return Err(DomainError::conflict(
            "sealed request attachment original does not match its preparation",
        ));
    }
    validate_sha256(&attachment.sha256)?;
    let max_bytes = match attachment.kind {
        RequestAttachmentKind::Photo => limits.max_photo_bytes,
        RequestAttachmentKind::Video => limits.max_video_bytes,
    };
    if attachment.size_bytes == 0 || attachment.size_bytes > max_bytes {
        return Err(DomainError::invalid_input(
            "validated request attachment exceeds its source byte limit",
        ));
    }
    validate_source_metadata(attachment.kind, image, video)
}

fn validate_source_metadata(
    kind: RequestAttachmentKind,
    image: Option<&RequestAttachmentImageMetadata>,
    video: Option<&RequestAttachmentVideoMetadata>,
) -> Result<(), DomainError> {
    let limits = RequestAttachmentLimits::default();
    match (kind, image, video) {
        (RequestAttachmentKind::Photo, Some(image), None)
            if image.width > 0
                && image.height > 0
                && u64::from(image.width)
                    .checked_mul(u64::from(image.height))
                    .is_some_and(|pixels| pixels <= limits.max_photo_pixels) => {}
        (RequestAttachmentKind::Video, None, Some(video))
            if video.width > 0
                && video.height > 0
                && video.duration_millis > 0
                && video.duration_millis
                    <= limits.max_video_duration_seconds.saturating_mul(1_000) => {}
        _ => {
            return Err(DomainError::invalid_input(
                "processed request attachment metadata does not match its kind",
            ));
        }
    }
    Ok(())
}

fn validate_derivatives(
    kind: RequestAttachmentKind,
    derivatives: &[RequestAttachmentDerivative],
) -> Result<(), DomainError> {
    if derivatives.is_empty() {
        return Err(DomainError::invalid_input(
            "processed request attachment requires a derivative",
        ));
    }
    let ids: BTreeSet<_> = derivatives.iter().map(|value| value.id.as_str()).collect();
    if ids.len() != derivatives.len() {
        return Err(DomainError::conflict(
            "request attachment derivative ids must be unique",
        ));
    }
    for derivative in derivatives {
        validate_required("request attachment derivative id", &derivative.id)?;
        validate_required(
            "request attachment derivative media type",
            &derivative.media_type,
        )?;
        validate_required(
            "request attachment derivative object key",
            &derivative.object.object_key,
        )?;
        validate_sha256(&derivative.object.sha256)?;
        if derivative.object.size_bytes == 0
            || matches!(
                (derivative.width, derivative.height),
                (Some(0), _) | (_, Some(0))
            )
            || derivative.width.is_some() != derivative.height.is_some()
        {
            return Err(DomainError::invalid_input(
                "request attachment derivative metadata is invalid",
            ));
        }
        let kind_and_media_type_match = match derivative.kind {
            RequestAttachmentDerivativeKind::ImagePreview
                if kind == RequestAttachmentKind::Photo =>
            {
                RequestAttachmentKind::from_media_type(&derivative.media_type)
                    == Some(RequestAttachmentKind::Photo)
                    && derivative.width.is_some()
                    && derivative.height.is_some()
                    && derivative.duration_millis.is_none()
            }
            RequestAttachmentDerivativeKind::VideoPoster
                if kind == RequestAttachmentKind::Video =>
            {
                RequestAttachmentKind::from_media_type(&derivative.media_type)
                    == Some(RequestAttachmentKind::Photo)
                    && derivative.width.is_some()
                    && derivative.height.is_some()
                    && derivative.duration_millis.is_none()
            }
            RequestAttachmentDerivativeKind::VideoPlayback
                if kind == RequestAttachmentKind::Video =>
            {
                derivative.media_type.eq_ignore_ascii_case("video/mp4")
                    && derivative.width.is_some()
                    && derivative.height.is_some()
                    && derivative
                        .duration_millis
                        .is_some_and(|duration| duration > 0)
            }
            _ => false,
        };
        if !kind_and_media_type_match {
            return Err(DomainError::invalid_input(
                "request attachment derivative kind and media type disagree",
            ));
        }
    }
    let kinds: BTreeSet<_> = derivatives.iter().map(|value| value.kind).collect();
    let required: BTreeSet<_> = match kind {
        RequestAttachmentKind::Photo => [RequestAttachmentDerivativeKind::ImagePreview]
            .into_iter()
            .collect(),
        RequestAttachmentKind::Video => [
            RequestAttachmentDerivativeKind::VideoPlayback,
            RequestAttachmentDerivativeKind::VideoPoster,
        ]
        .into_iter()
        .collect(),
    };
    if !required.is_subset(&kinds) {
        return Err(DomainError::invalid_input(
            "processed request attachment is missing required derivatives",
        ));
    }
    Ok(())
}

fn validate_part_receipts(
    attachment: &RequestAttachment,
    receipts: &[RequestAttachmentPartReceipt],
) -> Result<(), DomainError> {
    if receipts.is_empty() {
        return Err(DomainError::invalid_input(
            "request attachment requires upload part receipts",
        ));
    }
    let mut total = 0_u64;
    for (index, receipt) in receipts.iter().enumerate() {
        let expected_part = u32::try_from(index + 1)
            .map_err(|_| DomainError::invalid_input("too many request attachment parts"))?;
        if receipt.part_number != expected_part {
            return Err(DomainError::invalid_input(
                "request attachment parts must be contiguous and 1-based",
            ));
        }
        validate_part_range(attachment.size_bytes, receipt)?;
        total = total
            .checked_add(receipt.size_bytes)
            .ok_or_else(|| DomainError::invalid_input("request attachment part size overflow"))?;
    }
    if total != attachment.size_bytes {
        return Err(DomainError::conflict(
            "request attachment part bytes do not match the prepared size",
        ));
    }
    Ok(())
}

fn validate_failure_for_state(
    state: RequestAttachmentState,
    failure: Option<&RequestAttachmentFailure>,
) -> Result<(), DomainError> {
    if let Some(failure) = failure {
        let retryability_valid = match failure.code {
            RequestAttachmentFailureCode::InvalidMedia
            | RequestAttachmentFailureCode::CorruptMedia
            | RequestAttachmentFailureCode::UnsupportedMedia
            | RequestAttachmentFailureCode::MediaLimitExceeded => !failure.retryable,
            RequestAttachmentFailureCode::StorageUnavailable
            | RequestAttachmentFailureCode::Internal => failure.retryable,
            RequestAttachmentFailureCode::CodecFailed => true,
        };
        if !retryability_valid
            || failure.message.trim().is_empty()
            || failure.message.len() > 512
            || failure.message.chars().any(char::is_control)
        {
            return Err(DomainError::invalid_input(
                "request attachment failure code, message, and retryability disagree",
            ));
        }
    }
    match (state, failure) {
        (RequestAttachmentState::Failed, Some(failure)) if failure.retryable => Ok(()),
        (RequestAttachmentState::Rejected, Some(failure)) if !failure.retryable => Ok(()),
        (RequestAttachmentState::Failed | RequestAttachmentState::Rejected, _) => Err(
            DomainError::invalid_input("request attachment failure does not match its state"),
        ),
        (_, None) => Ok(()),
        (_, Some(_)) => Err(DomainError::invalid_input(
            "request attachment failure is only valid for failed or rejected state",
        )),
    }
}

fn validate_target(target: &RequestAttachmentTarget) -> Result<(), DomainError> {
    match target {
        RequestAttachmentTarget::Description => Ok(()),
        RequestAttachmentTarget::Discussion { discussion_id } => {
            if let Some(discussion_id) = discussion_id {
                validate_identifier("discussion id", discussion_id)
            } else {
                Ok(())
            }
        }
        RequestAttachmentTarget::Reply { discussion_id } => {
            validate_identifier("discussion id", discussion_id)
        }
    }
}

fn validate_binding_target(target: &RequestAttachmentBindingTarget) -> Result<(), DomainError> {
    match target {
        RequestAttachmentBindingTarget::Description => Ok(()),
        RequestAttachmentBindingTarget::Discussion { discussion_id } => {
            validate_identifier("discussion id", discussion_id)
        }
        RequestAttachmentBindingTarget::Reply {
            discussion_id,
            reply_id,
        } => {
            validate_identifier("discussion id", discussion_id)?;
            validate_identifier("reply id", reply_id)
        }
    }
}

fn validate_target_matches_binding(
    prepared: &RequestAttachmentTarget,
    binding: &RequestAttachmentBindingTarget,
) -> Result<(), DomainError> {
    let matches = match (prepared, binding) {
        (RequestAttachmentTarget::Description, RequestAttachmentBindingTarget::Description) => true,
        (
            RequestAttachmentTarget::Discussion {
                discussion_id: None,
            },
            RequestAttachmentBindingTarget::Discussion { .. },
        ) => true,
        (
            RequestAttachmentTarget::Discussion {
                discussion_id: Some(prepared),
            },
            RequestAttachmentBindingTarget::Discussion { discussion_id },
        ) => prepared == discussion_id,
        (
            RequestAttachmentTarget::Reply {
                discussion_id: prepared,
            },
            RequestAttachmentBindingTarget::Reply { discussion_id, .. },
        ) => prepared == discussion_id,
        _ => false,
    };
    if matches {
        Ok(())
    } else {
        Err(DomainError::forbidden(
            "request attachment was prepared for another target",
        ))
    }
}

fn validate_sha256(value: &str) -> Result<(), DomainError> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(DomainError::invalid_input(
            "request attachment sha256 must be 64 hexadecimal characters",
        ));
    }
    Ok(())
}

fn checked_usage(current: u64, added: u64) -> Result<u64, DomainError> {
    current
        .checked_add(added)
        .ok_or_else(|| DomainError::invalid_input("request attachment byte budget overflow"))
}

fn validate_identifier(label: &str, value: &str) -> Result<(), DomainError> {
    validate_required(label, value)?;
    if value.len() > REQUEST_ATTACHMENT_ID_MAX_BYTES || value.chars().any(char::is_control) {
        return Err(DomainError::invalid_input(format!(
            "{label} exceeds request attachment identifier limits"
        )));
    }
    Ok(())
}

fn validate_filename(filename: &str) -> Result<(), DomainError> {
    validate_required("filename", filename)?;
    if filename.len() > 255
        || filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(DomainError::invalid_input(
            "request attachment filename must be a 255-byte basename without control characters",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
