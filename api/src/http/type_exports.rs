use crate::http::{responses::*, routes};
use schemars::JsonSchema;
use scope_api_contract::*;
use std::{collections::BTreeMap, fs, path::Path};
use ts_rs::TS;

macro_rules! contract_declarations {
    ($config:expr; $($contract:ty $(=> $schema:ident)?),+ $(,)?) => {{
        let mut schemas = BTreeMap::new();
        $(contract_declarations!(@schema schemas, $contract $(, $schema)?);)+
        (vec![$(declaration::<$contract>($config)),+], schemas)
    }};
    (@schema $schemas:ident, $contract:ty) => {};
    (@schema $schemas:ident, $contract:ty, schema) => {{
        let (name, response_schema) = schema::<$contract>();
        assert!(
            $schemas.insert(name.clone(), response_schema).is_none(),
            "duplicate API response schema {name}",
        );
    }};
}

pub(crate) fn export_api_contract(output_path: &Path, schema_output_path: &Path) {
    let ts_config = ts_rs::Config::new().with_large_int("number");
    let (type_declarations, schemas) = contract_declarations!(
        &ts_config;
        ErrorCode,
        ErrorFields,
        ErrorResponse => schema,
        Visibility,
        RepositoryActor,
        RepositoryMemberPermissions,
        RepositoryInviteState,
        RepoLifecycleState,
        RepoChangeEvent => schema,
        FirstPushTokenStatus,
        FileChangeKind,
        ConfigVisibility,
        RepoConfig,
        RepoConfigVisibility,
        RepoConfigVisibilityRule,
        RepoConfigHistory,
        HistoryRewriteRequest,
        HistoryRewriteAction,
        RequestActorRole,
        RequestAudience,
        RequestState,
        RequestAttachmentTargetKind,
        RequestAttachmentTargetInput,
        RequestAttachmentKind,
        RequestAttachmentState,
        RequestAttachmentDerivativeKind,
        RequestAttachmentFailureCode,
        RequestAttachmentLimitsResponse => schema,
        PrepareRequestAttachmentRequest => schema,
        PrepareRequestAttachmentResponse => schema,
        RequestAttachmentTransferResponse,
        RequestAttachmentPartReceiptResponse,
        FinishRequestAttachmentRequest => schema,
        RetryRequestAttachmentRequest => schema,
        RequestAttachmentListResponse => schema,
        RequestAttachmentResponse => schema,
        RequestAttachmentImageMetadataResponse,
        RequestAttachmentVideoMetadataResponse,
        RequestAttachmentDerivativeResponse,
        RequestAttachmentFailureResponse,
        RequestAttachmentMediaTarget,
        CreateRequestAttachmentMediaGrantRequest => schema,
        CreateRequestAttachmentMediaGrantResponse => schema,
        RequestAttachmentMediaGrantMethod,
        RequestAttachmentMediaGrantClaims,
        RequestAttachmentUploadGrantClaims,
        GitOid,
        RequestEventKind,
        ProjectionPreviewAudience,
        AccountSessionResponse => schema,
        UserResponse,
        SessionResponse,
        SessionIdentity,
        SessionRepo,
        SessionCapabilities,
        DeviceLoginStatus,
        DeviceLoginStartResponse,
        DeviceLoginPollResponse,
        DeviceLoginCompleteResponse => schema,
        BrowserLoginStartRequest,
        BrowserLoginStartResponse,
        BrowserLoginCompleteResponse => schema,
        BrowserLoginExchangeRequest,
        CliSessionTokenResponse,
        CliExchangeGrantResponse => schema,
        CliExchangeGrantExchangeRequest,
        CliSessionsResponse => schema,
        CliSessionResponse,
        RepoSummaryResponse => schema,
        OwnerProfileResponse => schema,
        RepoRequestPermissionsResponse,
        CreateRepoRequest,
        UpdateRepoMetadataRequest,
        CreateRepoResponse,
        DeleteRepoResponse => schema,
        CreatePushIntentRequest,
        CreatePushIntentResponse,
        RepoInitResponse,
        RepoConfigResponse,
        FirstPushTokenResponse,
        GitPushTokenResponse,
        RepoFileResponse => schema,
        RepoFileContentRequest,
        RepoFileContentResponse => schema,
        RepositoryAccessResponse,
        RepositoryCollaborationResponse => schema,
        RepositoryMemberResponse => schema,
        RepositoryInviteResponse => schema,
        CreateRepositoryInviteRequest,
        CreateRepositoryInviteResponse => schema,
        UpdateRepositoryMemberRequest,
        RepositoryInviteLookupResponse => schema,
        AcceptRepositoryInviteResponse => schema,
        HistoryPageRequest,
        HistoryEntryRequest,
        HistoryEntryFileDiffRequest,
        RequestFileDiffRequest,
        ReviewFileContentResponse,
        ReviewFileDiffResponse => schema,
        HistoryPageResponse => schema,
        HistoryEntrySummaryResponse,
        HistoryEntryKind,
        HistoryFeed,
        HistoryEntryDetailResponse => schema,
        HistoryEntryFileResponse,
        NativeHistoryCommitResponse,
        HistoryVisibilitySummaryResponse,
        HistoryVisibilityChangeResponse,
        CommitFileResponse,
        ProjectionPreviewRequest,
        ProjectionPreviewResponse,
        ProjectionPreviewFileResponse,
        ProjectionPreviewCommitResponse,
        ProjectionPreviewCommitVisibilityResponse,
        ProjectionPreviewSummaryResponse,
        RequestQueueSection,
        RequestListResponse => schema,
        RequestDetailResponse => schema,
        CreateRequestRatingRequest,
        RequestRatingParticipantResponse,
        RequestRatingResponse => schema,
        RequestRatingsResponse => schema,
        RequestMutationResponse => schema,
        RequestListItemResponse,
        RequestSummaryResponse,
        RequestInviteeResponse,
        AddRequestInviteeRequest,
        RemoveRequestInviteeRequest,
        RequestInviteeMutationResponse => schema,
        LeaveRequestResponse => schema,
        RequestPermissionsResponse,
        RequestMergeabilityStatus,
        RequestMergeabilityResponse,
        RequestEventResponse,
        RequestEventPayload,
        RequestIdentityAuditFact,
        RequestActorSummaryResponse,
        RequestDiscussionStatus,
        RequestDiscussionReplyReferenceResponse,
        RequestDiscussionReplyResponse,
        RequestDiscussionSummaryResponse,
        RequestDiscussionAnchor,
        RequestDiscussionAnchorInput,
        RequestRevisionCommitResponse,
        RequestRevisionInspectionState,
        RequestRevisionResponse,
        RequestRevisionListResponse => schema,
        RequestDiscussionPageResponse => schema,
        RequestDiscussionRepliesPageResponse => schema,
        RequestDiscussionMutationResponse => schema,
        RequestDiscussionReplyMutationResponse => schema,
        RequestDiscussionChangesResponse => schema,
        RequestDiscussionReadResponse => schema,
        RequestActivityPageResponse => schema,
        RequestCloseResponse => schema,
        StartRequestRequest,
        SubmitRequestRequest,
        EditRequestIdentityRequest,
        CreateRequestDiscussionRequest,
        CreateRequestDiscussionReplyRequest,
        ReopenAndReplyRequest,
        MarkRequestDiscussionReadRequest,
        RepoChangeKind,
        RunChangeKind,
        RunState,
        RunResponse => schema,
        RepositoryRunState,
        RepositoryRunTrigger,
        RepositoryRunSummaryResponse,
        RepositoryRunJobState,
        RepositoryRunJobResponse,
        RepositoryRunJobDetailResponse,
        RepositoryRunAttemptState,
        RepositoryRunStepState,
        RepositoryRunTerminalReason,
        RepositoryRunCacheColdReason,
        RepositoryRunCachePreparation,
        RepositoryRunCacheFinalState,
        RepositoryRunCacheObservationResponse,
        RepositoryRunCacheSetupObservationResponse,
        RepositoryRunCacheResponse,
        RepositoryRunStepResponse,
        RepositoryRunAttemptResponse,
        RepositoryRunWorkflowResponse,
        RepositoryRunWorkflowListResponse => schema,
        RepositoryRunHistoryPageResponse => schema,
        RepositoryRunLogResponse,
        RepositoryRunDetailResponse => schema,
        RepositoryRunStepLogPageResponse => schema,
    );
    let declarations = [
        vec![generated_header()],
        type_declarations,
        vec![
            api_route_template_declarations(),
            api_path_builder_declaration(),
        ],
    ]
    .concat()
    .join("\n\n");

    fs::write(output_path, format!("{declarations}\n")).expect("write generated API types");
    let schema_document = serde_json::json!({
        "generated_from": "Rust API response/request types",
        "schemas": schemas,
    });
    fs::write(
        schema_output_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&schema_document)
                .expect("serialize generated API schemas"),
        ),
    )
    .expect("write generated API schemas");
}

fn api_route_template_declarations() -> String {
    let body = routes::WEB_ROUTE_TEMPLATES
        .iter()
        .map(|(name, path)| format!("  {name}: \"{path}\","))
        .collect::<Vec<_>>()
        .join("\n");
    format!("export const ApiRouteTemplates = {{\n{body}\n}} as const;")
}

fn api_path_builder_declaration() -> String {
    r#"export function buildApiPath(
  template: string,
  params: Readonly<Record<string, string>> = {},
): string {
  return template.replace(/\{([^}]+)\}/g, (_match, key: string) => {
    const value = params[key]
    if (value === undefined) throw new Error(`Missing API route parameter: ${key}`)
    return encodeURIComponent(value)
  })
}"#
    .to_string()
}

fn declaration<T: TS>(config: &ts_rs::Config) -> String {
    format!("export {}", T::decl(config))
}

fn schema<T: JsonSchema>() -> (String, serde_json::Value) {
    let name = T::schema_name().into_owned();
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .for_serialize()
        .into_generator()
        .into_root_schema_for::<T>();
    let mut schema = serde_json::to_value(schema).expect("serialize generated API schema");
    schema
        .as_object_mut()
        .expect("root API schema must be an object")
        .insert(
            "$id".to_string(),
            serde_json::Value::String(format!("scope://api/{name}")),
        );
    (name, schema)
}

fn generated_header() -> String {
    [
        "// This file is generated from Rust API response/request types.",
        "// Run `pnpm generate:api-contract` from web/ to update it.",
        "// Do not edit this file by hand.",
    ]
    .join("\n")
}
