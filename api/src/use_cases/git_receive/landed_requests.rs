use crate::{
    error::ApiError,
    git::{
        command::{git_is_ancestor, run_git_output},
        import::{ReceivePackUpdate, apply_receive_pack_update},
        projection_repo::verify_and_restore_projection,
    },
    state::AppState,
};
use scope_domain::{
    projection::project_graph,
    requests::{Request, lands_with_main},
};
use scope_postgres::db::LandedRequestCandidate;
use std::{collections::BTreeMap, path::Path};

#[tracing::instrument(skip_all, name = "use_case.git_receive.find_landed_requests", fields(scope.landed_request.count = tracing::field::Empty))]
pub(super) async fn landed_request_candidates(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &Path,
    update: &ReceivePackUpdate,
) -> Result<Vec<LandedRequestCandidate>, ApiError> {
    let repository_id = scope_domain::repository::repo_id(owner, repo_name);
    let candidates = state
        .metadata
        .requests()
        .requests_by_repo_id(&repository_id)
        .await?
        .into_iter()
        .filter(lands_with_main)
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let path = staging_repo.to_path_buf();
    let main_oid_for_scan = update.head_oid.clone();
    let mut landed = crate::git::blocking::run(move || {
        requests_carried_by(&path, &main_oid_for_scan, candidates)
    })
    .await?;
    if landed.is_empty() {
        return Ok(landed);
    }
    let committed = state
        .metadata
        .repositories()
        .repository(owner, repo_name)
        .await?
        .ok_or_else(|| ApiError::not_found("repo not found"))?;
    let incarnation = committed.incarnation();
    let mut proposed_repo = committed.clone();
    let mut proposed_update = update.clone();
    proposed_update.previous_config = Some(proposed_repo.repo_config.clone());
    apply_receive_pack_update(&mut proposed_repo, proposed_update)?;
    let mut view_heads = BTreeMap::new();
    for candidate in &mut landed {
        let parent = state
            .metadata
            .requests()
            .request_by_id(&candidate.request_id)
            .await?
            .ok_or_else(|| ApiError::conflict("landed request changed during preparation"))?;
        if parent.head_oid != candidate.head_oid {
            return Err(ApiError::conflict(
                "landed request changed during preparation",
            ));
        }
        if &parent.view != proposed_repo.repo_config.views().full()
            && !view_heads.contains_key(&parent.view)
        {
            let projection = project_graph(
                &proposed_repo.graph,
                &proposed_repo.visibility_change_sets,
                proposed_repo.repo_config.views(),
                &parent.view,
            );
            let head =
                verify_and_restore_projection(state, &committed, projection, staging_repo).await?;
            view_heads.insert(parent.view.clone(), head);
        }
        candidate.stack_children = crate::use_cases::request_stack_landing::prepare_stack_landing(
            state,
            &incarnation,
            &parent,
            staging_repo,
            &update.head_oid,
            view_heads.get(&parent.view).and_then(Option::as_deref),
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
