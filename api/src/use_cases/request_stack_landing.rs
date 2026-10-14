use crate::{
    error::ApiError,
    git::{
        command::{git_is_ancestor, run_git, run_git_output},
        request_refs::with_request_revision_store_repo,
    },
    state::AppState,
};
use scope_domain::{
    repository::RepositoryIncarnation,
    requests::{Request, RequestBase},
};
use scope_postgres::db::RequestStackChildFacts;
use std::path::Path;

pub(crate) async fn prepare_stack_landing(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    parent: &Request,
    staging_repo: &Path,
    main_oid: &str,
) -> Result<Vec<RequestStackChildFacts>, ApiError> {
    let children = state
        .metadata
        .requests()
        .request_stack_children(&parent.id)
        .await?;
    let mut facts = Vec::with_capacity(children.len());
    for child in children {
        let (snapshot_request, revision) = match state
            .metadata
            .requests()
            .latest_request_revision(&child.id)
            .await?
        {
            Some(revision) => (child.clone(), revision),
            None => {
                let RequestBase::Request { revision_id, .. } = &child.base else {
                    return Err(ApiError::conflict("child no longer has its parent pin"));
                };
                let revision = state
                    .metadata
                    .requests()
                    .request_revision(&parent.id, revision_id)
                    .await?
                    .ok_or_else(|| ApiError::conflict("saved parent revision is unavailable"))?;
                (parent.clone(), revision)
            }
        };
        let staging = staging_repo.to_path_buf();
        let child_head = child.head_oid.clone();
        let parent_head = parent.head_oid.clone();
        let main = main_oid.to_string();
        let review_base = child.base.review_base_oid().to_string();
        let ancestor = child.main_ancestor_oid.clone();
        let child_id = child.id.clone();
        let base_version = child.base_version;
        let fact = with_request_revision_store_repo(
            state,
            incarnation,
            &snapshot_request,
            &revision,
            move |raw_repo, _| {
                let internal_ref = format!("refs/scope/internal/stack-landing/{child_id}");
                let refspec = format!("{child_head}:{internal_ref}");
                run_git(
                    Some(&staging),
                    &[
                        "fetch",
                        "--no-tags",
                        raw_repo.to_string_lossy().as_ref(),
                        &refspec,
                    ],
                    "restoring child objects for parent landing",
                )?;
                run_git(
                    Some(&staging),
                    &["update-ref", "-d", &internal_ref],
                    "removing child landing preparation ref",
                )?;
                let parent_contains_pin = git_is_ancestor(
                    &staging,
                    &review_base,
                    &parent_head,
                    "checking landed parent contains child pin",
                )?;
                let output = run_git_output(
                    Some(&staging),
                    &["merge-base", &main, &child_head],
                    "reading child main ancestry after parent landing",
                )?;
                let contained_main_oid = match output.status.code() {
                    Some(0) => Some(
                        String::from_utf8(output.stdout)
                            .map_err(ApiError::internal)?
                            .trim()
                            .to_string(),
                    ),
                    Some(1) => None,
                    _ => {
                        return Err(ApiError::infrastructure_unavailable(
                            "cannot compute child main ancestry",
                        ));
                    }
                };
                let contained_main_descends_from_ancestor = match contained_main_oid.as_deref() {
                    Some(contained) => git_is_ancestor(
                        &staging,
                        &ancestor,
                        contained,
                        "checking monotonic child main ancestry",
                    )?,
                    None => false,
                };
                Ok(RequestStackChildFacts {
                    parent_head_oid: parent_head,
                    request_id: child_id,
                    head_oid: child_head,
                    base_version,
                    parent_contains_pin,
                    contained_main_oid,
                    contained_main_descends_from_ancestor,
                })
            },
        )
        .await?;
        facts.push(fact);
    }
    Ok(facts)
}
