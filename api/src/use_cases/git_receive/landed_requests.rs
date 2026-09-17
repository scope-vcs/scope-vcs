//! Completes open requests whose head a committed main push already carries.
//!
//! The push is authoritative by the time this runs, so nothing here can fail it.

use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
    repo_events::RepoChangeReason,
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, lands_with_main},
};
use scope_postgres::db::CompleteLandedRequestCommand;
use std::path::Path;

pub(super) async fn best_effort_complete_landed_requests(
    state: &AppState,
    repository_id: &str,
    incarnation: &RepositoryIncarnation,
    staging_repo: &Path,
    main_oid: &str,
    actor_user_id: &str,
) {
    match complete_landed_requests(state, repository_id, staging_repo, main_oid, actor_user_id)
        .await
    {
        Ok(0) => {}
        Ok(_) => {
            state
                .publish_request_summary_refresh(incarnation, RepoChangeReason::RequestMerged)
                .await;
        }
        Err(error) => tracing::warn!(
            repository_id,
            main_oid,
            error = %error.operator_diagnostic(),
            "completing requests carried by the main push failed"
        ),
    }
}

async fn complete_landed_requests(
    state: &AppState,
    repository_id: &str,
    staging_repo: &Path,
    main_oid: &str,
    actor_user_id: &str,
) -> Result<usize, ApiError> {
    let candidates = state
        .metadata
        .requests()
        .requests_by_repo_id(repository_id)
        .await?
        .into_iter()
        .filter(lands_with_main)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(0);
    }
    let landed = {
        let path = staging_repo.to_path_buf();
        let main_oid = main_oid.to_string();
        crate::git::blocking::run(move || requests_carried_by(&path, &main_oid, candidates)).await?
    };
    let mut completed = 0;
    for request in landed {
        let mutation = state
            .metadata
            .requests()
            .complete_landed_request(CompleteLandedRequestCommand {
                request_id: request.id.clone(),
                actor_user_id: actor_user_id.to_string(),
                merged_event_id: crate::persistence_ids::generate_prefixed_id(
                    "event_request_merged",
                )?,
                landed_head_oid: request.head_oid,
                main_oid: main_oid.to_string(),
                now_unix: crate::persistence::unix_now()?,
            })
            .await?;
        completed += usize::from(mutation.is_some());
    }
    Ok(completed)
}

fn requests_carried_by(
    repo: &Path,
    main_oid: &str,
    candidates: Vec<Request>,
) -> Result<Vec<Request>, ApiError> {
    let mut landed = Vec::new();
    for request in candidates {
        // A head that never reached this repository cannot be part of main.
        let head_commit = format!("{}^{{commit}}", request.head_oid);
        let present = run_git_output(
            Some(repo),
            &["cat-file", "-e", &head_commit],
            "checking request head presence",
        )?
        .status
        .success();
        if present
            && git_is_ancestor(
                repo,
                &request.head_oid,
                main_oid,
                "checking whether main carries a request head",
            )?
        {
            landed.push(request);
        }
    }
    Ok(landed)
}
