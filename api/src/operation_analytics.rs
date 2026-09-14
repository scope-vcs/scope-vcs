use crate::{error::ApiError, state::AppState};
use scope_product_analytics::{
    EventSource, OperationFailureReason, ProductActor, ProductEvent, ProductOperation,
};
use scope_service_runtime::http::ErrorKind;
use std::{future::Future, time::Instant};

/// A user-facing operation whose failures are reported to product analytics.
pub(crate) struct ObservedOperation<'a> {
    pub(crate) actor_user_id: &'a str,
    pub(crate) operation: ProductOperation,
    pub(crate) source: EventSource,
    pub(crate) repository_id: Option<&'a str>,
    pub(crate) request_id: Option<&'a str>,
}

impl ObservedOperation<'_> {
    /// Runs the operation and records a failure event when it errors. Timing starts here so
    /// every tracked operation measures duration the same way.
    pub(crate) async fn run<T>(
        self,
        state: &AppState,
        operation: impl Future<Output = Result<T, ApiError>>,
    ) -> Result<T, ApiError> {
        let started_at = Instant::now();
        let result = operation.await;
        if let Err(error) = &result {
            self.capture_failure(state, started_at, error);
        }
        result
    }

    fn capture_failure(self, state: &AppState, started_at: Instant, error: &ApiError) {
        let mut event = ProductEvent::operation_failed(
            ProductActor::User(self.actor_user_id),
            self.operation,
            failure_reason(error.kind),
            started_at.elapsed().as_millis().min(u64::MAX as u128) as u64,
        )
        .with_source(self.source);
        if let Some(repository_id) = self.repository_id {
            event = event.with_repository_id(repository_id);
        }
        if let Some(request_id) = self.request_id {
            event = event.with_request_id(request_id);
        }
        state.product_analytics.capture(event);
    }
}

fn failure_reason(kind: ErrorKind) -> OperationFailureReason {
    match kind {
        ErrorKind::Unauthorized => OperationFailureReason::AuthenticationRejected,
        ErrorKind::Forbidden => OperationFailureReason::PermissionDenied,
        ErrorKind::NotFound => OperationFailureReason::NotFound,
        ErrorKind::BadRequest | ErrorKind::PayloadTooLarge => OperationFailureReason::InvalidInput,
        ErrorKind::Conflict => OperationFailureReason::Conflict,
        ErrorKind::TooManyRequests => OperationFailureReason::RateLimited,
        ErrorKind::ServiceUnavailable => OperationFailureReason::Unavailable,
        ErrorKind::Internal => OperationFailureReason::Internal,
    }
}
