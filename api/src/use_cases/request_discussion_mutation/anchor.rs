use super::{DiscussionAnchorInput, MutationContext};
use crate::{
    error::ApiError,
    git::{import::run_git_output, request_refs::with_request_revision_store_repo},
    state::AppState,
    use_cases::request_revision_inspection::{
        DiffStatusValidationOrder, commit_belongs_to_revision, inspect_request_changes,
        request_changes,
    },
};
use scope_domain::{
    policy::{Policy, ScopePath},
    repository::access::RepositoryAccess,
    requests::{RequestDiscussionAnchor, RequestRevision},
};
use std::{collections::BTreeSet, path::Path as FsPath};

pub(super) async fn validate(
    state: &AppState,
    _owner: &str,
    _repo_name: &str,
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

pub(super) async fn visible_commits(
    state: &AppState,
    _owner: &str,
    _repo_name: &str,
    context: &MutationContext,
    anchor: Option<&RequestDiscussionAnchor>,
) -> BTreeSet<(String, String)> {
    let Some(anchor) = anchor else {
        return BTreeSet::new();
    };
    let Some(commit_oid) = anchor.commit_oid.as_deref() else {
        return BTreeSet::new();
    };
    if context.access.can_read_private_files {
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
        let access = context.access;
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
    let (paths, has_hidden) = commit_paths(raw_repo, policy, access, commit_oid)?;
    if has_hidden {
        return Err(ApiError::not_found("request revision commit not found"));
    }
    Ok(paths)
}

fn commit_is_fully_visible(
    raw_repo: &FsPath,
    policy: &Policy,
    access: RepositoryAccess,
    revision: &RequestRevision,
    commit_oid: &str,
) -> Result<bool, ApiError> {
    if !commit_belongs_to_revision(raw_repo, revision, commit_oid)? {
        return Ok(false);
    }
    commit_paths(raw_repo, policy, access, commit_oid).map(|(_, has_hidden)| !has_hidden)
}

fn commit_paths(
    raw_repo: &FsPath,
    policy: &Policy,
    access: RepositoryAccess,
    commit_oid: &str,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let parents = run_git_output(
        Some(raw_repo),
        &["show", "-s", "--format=%P", commit_oid],
        "reading request commit identity",
    )?;
    if !parents.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading request commit identity: {}",
            String::from_utf8_lossy(&parents.stderr).trim()
        )));
    }
    let parent = String::from_utf8(parents.stdout)
        .map_err(ApiError::bad_request)?
        .split_whitespace()
        .next()
        .map(str::to_string)
        .ok_or_else(|| ApiError::conflict("request revision commit must have a parent"))?;
    let changes = request_changes(raw_repo, &parent, commit_oid, None)?;

    parse_commit_paths(&changes, policy, access)
}

fn parse_commit_paths(
    changes: &[u8],
    policy: &Policy,
    access: RepositoryAccess,
) -> Result<(BTreeSet<ScopePath>, bool), ApiError> {
    let inspected = inspect_request_changes(
        changes,
        policy,
        access,
        DiffStatusValidationOrder::BeforePath,
    )?;
    Ok((
        inspected
            .files
            .into_iter()
            .map(|file| file.scope_path)
            .collect(),
        inspected.hidden,
    ))
}

fn normalized_scope_path(path: &str) -> Result<ScopePath, ApiError> {
    let path = ScopePath::parse(format!("/{}", path.trim_start_matches('/')))
        .map_err(ApiError::bad_request)?;
    if path == ScopePath::root() {
        return Err(ApiError::bad_request("file path is required"));
    }
    Ok(path)
}

fn canonical_git_oid(oid: String) -> Result<String, ApiError> {
    if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::bad_request(
            "Git OID must be exactly 40 hexadecimal characters",
        ));
    }
    Ok(oid.to_ascii_lowercase())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod identity_tests;
