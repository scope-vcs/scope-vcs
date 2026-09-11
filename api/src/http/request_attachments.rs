use crate::persistence_ids::generate_prefixed_id;
use crate::use_cases::request_access::visible_request;
use crate::{
    auth::scope::require_scope_user, error::ApiError, http::requests::repo_metadata_and_access,
    persistence::unix_now, state::AppState,
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::attachments::*;
use scope_domain::requests::attachments::RequestAttachmentLimits;
use scope_postgres::db::{
    FinishRequestAttachmentUploadCommand, PrepareRequestAttachmentCommand, RequestMediaObjectTarget,
};

type RequestPath = (String, String, String);
type AttachmentPath = (String, String, String, String);

// The path is checked independently of the attachment lookup, so an authorized
// attachment cannot be addressed through a different repository URL.
async fn request_viewer(
    state: &AppState,
    headers: &HeaderMap,
    owner: &str,
    repo_name: &str,
    request_id: &str,
) -> Result<Option<String>, ApiError> {
    let (repo, access, viewer) = repo_metadata_and_access(state, headers, owner, repo_name).await?;
    visible_request(
        state,
        &repo.record.id,
        access,
        viewer.as_deref(),
        request_id,
    )
    .await?;
    Ok(viewer)
}

pub(crate) async fn limits(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id)): Path<RequestPath>,
) -> Result<Json<RequestAttachmentLimitsResponse>, ApiError> {
    request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    Ok(Json(RequestAttachmentLimits::default().into()))
}

pub(crate) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id)): Path<RequestPath>,
) -> Result<Json<RequestAttachmentListResponse>, ApiError> {
    let viewer = request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let attachments = state
        .metadata
        .media()
        .list_request_attachments_for_viewer(&request_id, viewer.as_deref())
        .await?;
    Ok(Json(RequestAttachmentListResponse {
        attachments: attachments.into_iter().map(Into::into).collect(),
    }))
}

pub(crate) async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id, attachment_id)): Path<AttachmentPath>,
) -> Result<Json<RequestAttachmentResponse>, ApiError> {
    let viewer = request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let attachment = state
        .metadata
        .media()
        .request_attachment_for_viewer(&request_id, &attachment_id, viewer.as_deref())
        .await?
        .ok_or_else(|| ApiError::not_found("attachment not found"))?;
    Ok(Json(attachment.into()))
}

pub(crate) async fn prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id)): Path<RequestPath>,
    Json(input): Json<PrepareRequestAttachmentRequest>,
) -> Result<Json<PrepareRequestAttachmentResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let now = unix_now()?;
    let limits = RequestAttachmentLimits::default();
    let prepared = state
        .metadata
        .media()
        .prepare_request_attachment(
            PrepareRequestAttachmentCommand {
                attachment_id: generate_prefixed_id("attachment_")?,
                upload_id: generate_prefixed_id("upload_")?,
                operation_id: input.operation_id,
                request_id,
                actor_user_id: user.id,
                target: input.target.try_into()?,
                filename: input.filename,
                declared_media_type: input.declared_media_type,
                size_bytes: input.size_bytes,
                sha256: input.sha256,
                now_unix: now,
            },
            limits,
        )
        .await?;
    let expires_at_unix = state
        .media_grants
        .expires_at(now)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let grant = state
        .media_grants
        .issue_upload(&RequestAttachmentUploadGrantClaims {
            attachment_id: prepared.attachment.id.clone(),
            repository_id: prepared.attachment.repository_id.clone(),
            request_id: prepared.attachment.request_id.clone(),
            uploader_user_id: prepared.attachment.uploader_user_id.clone(),
            upload_id: prepared.upload_id.clone(),
            expires_at_unix,
        })
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    Ok(Json(PrepareRequestAttachmentResponse {
        attachment: prepared.attachment.into(),
        transfer: RequestAttachmentTransferResponse {
            upload_id: prepared.upload_id,
            media_base_url: state.media_grants.endpoint().into(),
            grant,
            expires_at_unix,
            preferred_part_bytes: limits.preferred_part_bytes,
            max_concurrent_parts: limits.max_concurrent_parts,
            acknowledged_parts: prepared
                .acknowledged_parts
                .into_iter()
                .map(Into::into)
                .collect(),
        },
    }))
}

pub(crate) async fn finish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id, attachment_id)): Path<AttachmentPath>,
    Json(input): Json<FinishRequestAttachmentRequest>,
) -> Result<Json<RequestAttachmentResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let attachment = state
        .metadata
        .media()
        .finish_request_attachment_upload(FinishRequestAttachmentUploadCommand {
            request_id,
            attachment_id,
            upload_id: input.upload_id,
            actor_user_id: user.id,
            parts: input.parts.into_iter().map(Into::into).collect(),
            now_unix: unix_now()?,
        })
        .await?;
    Ok(Json(attachment.into()))
}

pub(crate) async fn retry(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id, attachment_id)): Path<AttachmentPath>,
    Json(input): Json<RetryRequestAttachmentRequest>,
) -> Result<Json<RequestAttachmentResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let attachment = state
        .metadata
        .media()
        .retry_request_attachment_processing(
            &request_id,
            &attachment_id,
            &user.id,
            &input.operation_id,
            unix_now()?,
        )
        .await?;
    Ok(Json(attachment.into()))
}

pub(crate) async fn media_grant(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo, request_id, attachment_id)): Path<AttachmentPath>,
    Json(input): Json<CreateRequestAttachmentMediaGrantRequest>,
) -> Result<Json<CreateRequestAttachmentMediaGrantResponse>, ApiError> {
    let viewer = request_viewer(&state, &headers, &owner, &repo, &request_id).await?;
    let media = state.metadata.media();
    let attachment = media
        .request_attachment_for_viewer(&request_id, &attachment_id, viewer.as_deref())
        .await?
        .ok_or_else(|| ApiError::not_found("attachment not found"))?;
    let target = match &input.target {
        RequestAttachmentMediaTarget::Original => RequestMediaObjectTarget::Original,
        RequestAttachmentMediaTarget::Derivative { derivative_id } => {
            RequestMediaObjectTarget::Derivative(derivative_id)
        }
    };
    media
        .authorized_media_manifest(&request_id, &attachment_id, viewer.as_deref(), target)
        .await?
        .ok_or_else(|| ApiError::not_found("attachment media is not available"))?;
    let expires_at_unix = state
        .media_grants
        .expires_at(unix_now()?)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let grant = state
        .media_grants
        .issue_media(&RequestAttachmentMediaGrantClaims {
            attachment_id: attachment_id.clone(),
            repository_id: attachment.repository_id,
            request_id,
            viewer_user_id: viewer,
            method: RequestAttachmentMediaGrantMethod::Get,
            target: input.target.clone(),
            expires_at_unix,
        })
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    let media_url = state
        .media_grants
        .media_url(&attachment_id, &input.target, &grant)
        .map_err(|error| ApiError::internal_message(error.to_string()))?;
    Ok(Json(CreateRequestAttachmentMediaGrantResponse {
        media_url,
        grant,
        expires_at_unix,
    }))
}
