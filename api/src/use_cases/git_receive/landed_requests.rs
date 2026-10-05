use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
    state::AppState,
};
use scope_domain::requests::{Request, lands_with_main};
use scope_postgres::db::LandedRequestCandidate;
use std::path::Path;

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
    let main_oid = main_oid.to_string();
    crate::git::blocking::run(move || requests_carried_by(&path, &main_oid, candidates)).await
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
            });
        }
    }
    Ok(landed)
}
