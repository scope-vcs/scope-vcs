use super::{DiscussionAnchorInput, MutationContext};
use crate::{
    error::ApiError,
    git::request_refs::with_request_revision_store_repo,
    state::AppState,
    use_cases::request_revision_inspection::{
        commit_is_fully_visible, normalized_scope_path, visible_commit_paths,
    },
};
use scope_domain::requests::RequestDiscussionAnchor;
use std::collections::BTreeSet;

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
    let commit_oid = anchor.commit_oid.map(canonical_git_oid).transpose()?;
    if let Some(commit_oid) = commit_oid.as_deref() {
        let policy = state
            .metadata
            .repositories()
            .repository_policy(&context.repo)
            .await?;
        let access = context.repo.access;
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

pub(super) async fn visible_commits(
    state: &AppState,
    context: &MutationContext,
    anchor: Option<&RequestDiscussionAnchor>,
) -> BTreeSet<(String, String)> {
    let Some(anchor) = anchor else {
        return BTreeSet::new();
    };
    let Some(commit_oid) = anchor.commit_oid.as_deref() else {
        return BTreeSet::new();
    };
    if context.repo.access.can_read_private_files {
        return BTreeSet::from([(anchor.revision_id.clone(), commit_oid.to_string())]);
    }
    let result = async {
        let revision = state
            .metadata
            .requests()
            .request_revision(&context.request.id, &anchor.revision_id)
            .await?
            .ok_or_else(|| ApiError::not_found("request revision not found"))?;
        let policy = state
            .metadata
            .repositories()
            .repository_policy(&context.repo)
            .await?;
        let access = context.repo.access;
        let commit_oid = commit_oid.to_string();
        let visible = with_request_revision_store_repo(
            state,
            &context.repo.incarnation(),
            &context.request,
            &revision,
            move |raw_repo, revision| {
                commit_is_fully_visible(raw_repo, &policy, access, revision, &commit_oid)
            },
        )
        .await?;
        Ok::<_, ApiError>(visible)
    }
    .await;
    match result {
        Ok(true) => BTreeSet::from([(anchor.revision_id.clone(), commit_oid.to_string())]),
        Ok(false) => BTreeSet::new(),
        Err(error) => {
            tracing::warn!(
                request_id = %context.request.id,
                revision_id = %anchor.revision_id,
                error = ?error,
                "redacting discussion anchor because request revision inspection failed"
            );
            BTreeSet::new()
        }
    }
}

fn canonical_git_oid(oid: String) -> Result<String, ApiError> {
    if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "Git OID must be exactly 40 hexadecimal characters",
        ));
    }
    Ok(oid.to_ascii_lowercase())
}
