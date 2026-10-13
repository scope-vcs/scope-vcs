use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
    state::AppState,
};
use scope_domain::requests::{Request, lands_with_main};
use scope_postgres::db::LandedRequestCandidate;
use std::path::Path;

#[tracing::instrument(skip_all, name = "use_case.git_receive.find_landed_requests", fields(scope.landed_request.count = tracing::field::Empty))]
pub(super) async fn landed_request_candidates(
    state: &AppState,
    repository_id: &str,
    staging_repo: &Path,
    main_oid: &str,
) -> Result<Vec<LandedRequestCandidate>, ApiError> {
    let candidates = state
        .metadata
        .requests()
        .requests_by_repo_id(repository_id)
        .await?
        .into_iter()
        .filter(lands_with_main)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let path = staging_repo.to_path_buf();
    let main_oid_for_scan = main_oid.to_string();
    let mut landed = crate::git::blocking::run(move || {
        requests_carried_by(&path, &main_oid_for_scan, candidates)
    })
    .await?;
    let incarnation = state
        .metadata
        .repositories()
        .repository_record(repository_id)
        .await?
        .ok_or_else(|| ApiError::not_found("repo not found"))?
        .incarnation();
    for candidate in &mut landed {
        let parent = state
            .metadata
            .requests()
            .request_by_id(&candidate.request_id)
            .await?
            .ok_or_else(|| ApiError::conflict("landed request changed during preparation"))?;
        candidate.stack_children = crate::use_cases::request_stack_landing::prepare_stack_landing(
            state,
            &incarnation,
            &parent,
            staging_repo,
            main_oid,
        )
        .await?;
    }
    tracing::Span::current().record("scope.landed_request.count", landed.len());
    Ok(landed)
}

fn requests_carried_by(
    repo: &Path,
    main_oid: &str,
    candidates: Vec<Request>,
) -> Result<Vec<LandedRequestCandidate>, ApiError> {
    let mut landed = Vec::new();
    for request in candidates {
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
            landed.push(LandedRequestCandidate {
                request_id: request.id,
                head_oid: request.head_oid,
                stack_children: Vec::new(),
            });
        }
    }
    Ok(landed)
}
