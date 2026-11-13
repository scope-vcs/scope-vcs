use super::{
    Request, RequestAutoMergeIntent, RequestAutoMergeMutation, RequestAutoMergeStopReason,
    RequestCheckEvaluation, stop_request_auto_merge,
};
use crate::{error::DomainError, requests::RequestCheckEvaluationState};

pub fn stop_request_auto_merge_for_check_evaluation(
    request: &Request,
    intent: &RequestAutoMergeIntent,
    evaluation: &RequestCheckEvaluation,
    event_id: String,
) -> Result<Option<RequestAutoMergeMutation>, DomainError> {
    if evaluation.state != RequestCheckEvaluationState::ConfigurationError
        || evaluation.request_id != intent.request_id
        || evaluation.head_oid != intent.head_oid
        || evaluation.revision_id != intent.revision_id
    {
        return Ok(None);
    }
    stop_request_auto_merge(
        request,
        intent,
        RequestAutoMergeStopReason::ChecksConfigurationError,
        event_id,
        evaluation
            .updated_at_unix
            .max(request.updated_at_unix)
            .max(intent.updated_at_unix),
    )
    .map(Some)
}
