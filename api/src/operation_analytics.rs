use crate::{error::ApiError, state::AppState};
use scope_product_analytics::{
    EventSource, OperationFailureReason, ProductActor, ProductEvent, ProductOperation,
};
use scope_service_runtime::http::ErrorKind;
use std::time::Instant;

pub(crate) struct OperationFailureContext<'a> {
    pub(crate) actor_user_id: &'a str,
    pub(crate) operation: ProductOperation,
    pub(crate) source: EventSource,
    pub(crate) repository_id: Option<&'a str>,
    pub(crate) request_id: Option<&'a str>,
    pub(crate) started_at: Instant,
}

pub(crate) fn capture_operation_failure(
    state: &AppState,
    context: OperationFailureContext<'_>,
    error: &ApiError,
) {
    let mut event = ProductEvent::operation_failed(
        ProductActor::User(context.actor_user_id),
        context.operation,
        failure_reason(error.kind),
        context
            .started_at
            .elapsed()
            .as_millis()
            .min(u64::MAX as u128) as u64,
    )
    .with_source(context.source);
    if let Some(repository_id) = context.repository_id {
        event = event.with_repository_id(repository_id);
    }
    if let Some(request_id) = context.request_id {
        event = event.with_request_id(request_id);
    }
    state.product_analytics.capture(event);
}

fn failure_reason(kind: ErrorKind) -> OperationFailureReason {
    match kind {
        ErrorKind::Unauthorized => OperationFailureReason::AuthenticationRejected,
        ErrorKind::Forbidden | ErrorKind::NotFound => OperationFailureReason::PermissionDenied,
        ErrorKind::BadRequest | ErrorKind::PayloadTooLarge => OperationFailureReason::InvalidInput,
        ErrorKind::Conflict => OperationFailureReason::Conflict,
        ErrorKind::TooManyRequests => OperationFailureReason::RateLimited,
        ErrorKind::ServiceUnavailable => OperationFailureReason::Unavailable,
        ErrorKind::Internal => OperationFailureReason::Internal,
    }
}
