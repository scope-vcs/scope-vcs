//! What a request's checks currently say about merging it.

use crate::{error::ApiError, state::AppState};
use scope_domain::{
    requests::{Request, RequestCheckEvaluation, RequestChecksOutcome, request_checks_outcome},
    runs::run::RunState,
};
use scope_postgres::db::RequestListRow;
use std::collections::HashMap;

pub(crate) async fn checks_outcome(
    state: &AppState,
    request: &Request,
) -> Result<RequestChecksOutcome, ApiError> {
    let evaluation = state
        .metadata
        .requests()
        .request_check_evaluation(&request.id, &request.head_oid)
        .await?;
    let run_states = run_states(state, evaluation.iter()).await?;
    Ok(request_checks_outcome(
        &request.id,
        &request.head_oid,
        evaluation.as_ref(),
        &run_states,
    ))
}

/// The outcome for every listed request, keyed by request id, loaded in two queries.
pub(crate) async fn checks_outcomes(
    state: &AppState,
    requests: &[RequestListRow],
) -> Result<HashMap<String, RequestChecksOutcome>, ApiError> {
    let heads = requests
        .iter()
        .map(|request| (request.id.clone(), request.head_oid.clone()))
        .collect::<Vec<_>>();
    let evaluations = state
        .metadata
        .requests()
        .request_check_evaluations(&heads)
        .await?;
    let run_states = run_states(state, evaluations.iter()).await?;
    Ok(requests
        .iter()
        .map(|row| {
            let evaluation = evaluations
                .iter()
                .find(|evaluation| evaluation.request_id == row.id);
            (
                row.id.clone(),
                request_checks_outcome(&row.id, &row.head_oid, evaluation, &run_states),
            )
        })
        .collect())
}

async fn run_states<'a>(
    state: &AppState,
    evaluations: impl Iterator<Item = &'a RequestCheckEvaluation>,
) -> Result<Vec<(String, RunState)>, ApiError> {
    let run_ids = evaluations
        .flat_map(|evaluation| evaluation.run_ids().map(str::to_string))
        .collect::<Vec<_>>();
    Ok(state
        .metadata
        .runs()
        .runs_by_ids(&run_ids)
        .await?
        .into_iter()
        .map(|run| (run.id, run.state))
        .collect())
}
