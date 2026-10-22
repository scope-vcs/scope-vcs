use super::{DiscussionAnchorInput, MutationContext};
use crate::{
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    state::AppState,
    use_cases::{
        request_diff_file::{
            RequestBlobContent, RequestDiffRange, request_blob_content, request_diff_file,
            request_diff_side_oid,
        },
        request_revision_inspection::{commit_belongs_to_revision, request_commit_visible_paths},
        scope_path_input::normalized_scope_path,
    },
};
use scope_api_contract::GitOid;
use scope_domain::{
    policy::{Policy, ScopePath},
    repository::access::{RepositoryAccess, RepositoryAccessContext},
    requests::{
        Request, RequestDiscussionAnchor, RequestDiscussionLines, RequestLineRange,
        RequestLineTarget, RequestRevision,
    },
    views::Views,
};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(super) async fn validate(
    state: &AppState,
    context: &MutationContext,
    input: DiscussionAnchorInput,
) -> Result<RequestDiscussionAnchor, ApiError> {
    let revision = state
        .metadata
        .requests()
        .request_revision(&context.request.id, &input.revision_id)
        .await?
        .ok_or_else(|| ApiError::not_found("request revision not found"))?;
    let lines = input
        .lines
        .map(|lines| {
            Ok::<_, ApiError>(RequestDiscussionLines {
                side: lines.side.into(),
                range: RequestLineRange::new(lines.start_line, lines.end_line)?,
            })
        })
        .transpose()?;
    let anchor = RequestDiscussionAnchor {
        revision_id: revision.id.clone(),
        commit_oid: input
            .commit_oid
            .map(|oid| {
                GitOid::try_from(oid)
                    .map(String::from)
                    .map_err(ApiError::bad_request)
            })
            .transpose()?,
        path: input
            .path
            .map(|path| normalized_scope_path(&path))
            .transpose()?,
        lines,
    };
    ensure_target(
        state,
        &context.repo,
        &context.access,
        &context.request,
        &revision,
        &anchor,
    )
    .await?;
    Ok(anchor)
}

pub(super) async fn ensure_target(
    state: &AppState,
    repo_context: &RepositoryAccessContext,
    access: &RepositoryAccess,
    request: &Request,
    revision: &RequestRevision,
    anchor: &RequestDiscussionAnchor,
) -> Result<(), ApiError> {
    anchor.validate()?;
    if anchor.commit_oid.is_none() && anchor.path.is_none() {
        return Ok(());
    }
    let repo = state
        .metadata
        .repositories()
        .repository_policy_config(repo_context)
        .await?;
    let inspection_state = state.clone();
    let access = access.clone();
    let checked = anchor.clone();
    with_request_revision_store_repo(
        state,
        &repo_context.incarnation(),
        request,
        revision,
        move |raw_repo, revision| {
            let path = match (checked.path.as_ref(), checked.commit_oid.as_deref()) {
                (Some(path), _) => path,
                (None, None) => return Ok(()),
                (None, Some(commit_oid)) => {
                    let policy = crate::use_cases::request_inspection_policy::resolve_commit(
                        &inspection_state,
                        &repo,
                        &access.view,
                        raw_repo,
                        commit_oid,
                    )?;
                    visible_commit_paths(
                        raw_repo,
                        &policy,
                        &repo.context.views,
                        access,
                        revision,
                        commit_oid,
                    )?;
                    return Ok(());
                }
            };
            let file = request_diff_file(
                &inspection_state,
                &repo,
                &access,
                raw_repo,
                revision,
                &RequestDiffRange::of_anchor(&checked),
                path,
            )?
            .ok_or_else(|| {
                ApiError::bad_request(
                    "request discussion path is not changed by the selected diff range",
                )
            })?;
            if let Some(lines) = checked.lines {
                let content = request_diff_side_oid(&file, lines.side)
                    .map(|oid| request_blob_content(raw_repo, oid))
                    .transpose()?;
                lines.ensure_target(match &content {
                    None => RequestLineTarget::Missing,
                    Some(RequestBlobContent::Binary { .. }) => RequestLineTarget::Binary,
                    Some(RequestBlobContent::Text(text)) => RequestLineTarget::Text(text),
                })?;
            }
            Ok(())
        },
    )
    .await
}

fn visible_commit_paths(
    raw_repo: &FsPath,
    policy: &Policy,
    views: &Views,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<BTreeSet<ScopePath>, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    let (paths, has_hidden) =
        request_commit_visible_paths(raw_repo, policy, views, &access, commit_oid)?;
    if has_hidden {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    Ok(paths)
}

#[cfg(test)]
mod identity_tests;
