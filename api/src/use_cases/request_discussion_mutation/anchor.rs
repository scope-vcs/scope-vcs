use super::{DiscussionAnchorInput, MutationContext};
use crate::{
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    state::AppState,
    use_cases::{
        request_revision_inspection::{commit_belongs_to_revision, request_commit_visible_paths},
        scope_path_input::normalized_scope_path,
    },
};
use scope_api_contract::GitOid;
use scope_domain::{
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
    requests::{RequestDiscussionAnchor, RequestRevision},
};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(super) async fn validate(
    state: &AppState,
    context: &MutationContext,
    anchor: DiscussionAnchorInput,
) -> Result<RequestDiscussionAnchor, ApiError> {
    if anchor.path.is_some() && anchor.commit_oid.is_none() {
        return Err(ApiError::bad_request(
            "request discussion path requires a commit",
        ));
    }
    let revision = state
        .metadata
        .requests()
        .request_revision(&context.request.id, &anchor.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let path = anchor
        .path
        .map(|path| normalized_scope_path(&path))
        .transpose()?;
    let commit_oid = anchor
        .commit_oid
        .map(|oid| {
            GitOid::try_from(oid)
                .map(String::from)
                .map_err(ApiError::bad_request)
        })
        .transpose()?;
    if let Some(commit_oid) = commit_oid.as_deref() {
        let policy = state
            .metadata
            .repositories()
            .repository_policy(&context.repo)
            .await?;
        let access = context.access;
        let commit_oid = commit_oid.to_string();
        let visible_paths = with_request_revision_store_repo(
            state,
            &context.repo.incarnation(),
            &context.request,
            &revision,
            move |raw_repo, revision| {
                visible_commit_paths(raw_repo, &policy, access, revision, &commit_oid)
            },
        )
        .await?;
        if let Some(path) = path.as_ref()
            && !visible_paths.contains(path)
        {
            return Err(ApiError::bad_request(
                "request discussion path is not changed by the selected commit",
            ));
        }
    }
    Ok(RequestDiscussionAnchor {
        revision_id: revision.id,
        commit_oid,
        path,
    })
}

fn visible_commit_paths(
    raw_repo: &FsPath,
    policy: &Policy,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<BTreeSet<ScopePath>, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let (paths, has_hidden) = request_commit_visible_paths(raw_repo, policy, access, commit_oid)?;
    if has_hidden {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    Ok(paths)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests;
