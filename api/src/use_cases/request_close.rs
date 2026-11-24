use crate::after_commit::AfterCommit;
use crate::{
    error::ApiError, persistence::unix_now, persistence_ids::generate_prefixed_id,
    repo_events::RepoChangeReason, state::AppState,
};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{
        CloseRequestInput, CloseRequestMutation, Request, RequestAutoMergeStopReason,
        RequestCloseAs, RequestCloseReason, close_request as close, request_actor_role,
        request_auto_merge_event_id, stop_request_auto_merge,
    },
};
use scope_postgres::{db::RequestStore, error::PostgresError};
use scope_product_analytics::{ProductEvent, RequestCloseOutcome};

#[tracing::instrument(skip_all, name = "use_case.request.close")]
pub(crate) async fn close_request(
    state: &AppState,
    repo: &RepositoryAccessContext,
    request: &Request,
    user_id: &str,
    reason: Option<RequestCloseReason>,
    named_request_id: Option<String>,
) -> Result<CloseRequestMutation, ApiError> {
    let mutation = crate::use_cases::request_close::persist_close(
        &state.metadata.requests(),
        CloseRequestCommand {
            request_id: request.id.clone(),
            actor_user_id: user_id.to_string(),
            reason,
            named_request_id,
            event_id: generate_prefixed_id("event_request_closed_")?,
            now_unix: unix_now()?,
        },
    )
    .await?;
    let (outcome, reason) = match &mutation {
        CloseRequestMutation::DeletedDraft { .. } => (
            RequestCloseOutcome::DraftDeleted,
            RepoChangeReason::RequestDeleted,
        ),
        CloseRequestMutation::Closed { .. } => {
            (RequestCloseOutcome::Closed, RepoChangeReason::RequestClosed)
        }
    };
    state.after_commit([
        AfterCommit::Analytics(ProductEvent::request_closed(
            user_id,
            repo.incarnation().incarnation_id(),
            &request.id,
            request.view.clone(),
            request_actor_role(repo.access.clone()),
            outcome,
        )),
        AfterCommit::request_summary(&repo.incarnation(), reason),
    ]);
    Ok(mutation)
}

#[derive(Clone, Debug)]
pub(crate) struct CloseRequestCommand {
    pub(crate) request_id: String,
    pub(crate) actor_user_id: String,
    pub(crate) reason: Option<RequestCloseReason>,
    pub(crate) named_request_id: Option<String>,
    pub(crate) event_id: String,
    pub(crate) now_unix: u64,
}

pub(crate) async fn persist_close(
    store: &RequestStore,
    command: CloseRequestCommand,
) -> Result<CloseRequestMutation, PostgresError> {
    let tx = store
        .begin_request_close(&command.request_id, &command.actor_user_id)
        .await?;
    let named = tx
        .named_requests(command.named_request_id.as_slice())
        .await?
        .pop();
    let reason = RequestCloseAs::new(command.reason, named)?;
    let facts = tx.close_facts().await?;
    let now_unix = command.now_unix;
    let mut mutation = close(
        tx.request().clone(),
        facts.events,
        facts.revisions,
        CloseRequestInput {
            request_id: command.request_id,
            actor_user_id: command.actor_user_id,
            actor_is_maintainer: tx.repository().access.is_maintainer(),
            reason,
            event_id: command.event_id,
            now_unix,
        },
    )?;
    let stopped = match (&mutation, &facts.active_auto_merge) {
        (CloseRequestMutation::Closed { request, .. }, Some(locked)) => {
            Some(stop_request_auto_merge(
                request,
                locked.intent(),
                RequestAutoMergeStopReason::RequestClosed,
                request_auto_merge_event_id("stopped", &locked.intent().id),
                now_unix,
            )?)
        }
        _ => None,
    };
    tx.save_close(
        &mutation,
        facts.active_auto_merge.zip(stopped.as_ref()),
        now_unix,
    )
    .await?;
    if let (CloseRequestMutation::Closed { request, .. }, Some(stopped)) = (&mut mutation, stopped)
    {
        *request = stopped.request;
    }
    tx.commit().await?;
    Ok(mutation)
}

#[cfg(test)]
mod tests;
