use crate::{
    error::ApiError,
    git::command::{git_is_ancestor, run_git_output},
};
use scope_domain::requests::{RequestAudience, RequestRevisionGitFacts};
use std::path::Path as FsPath;

pub(super) fn thin_snapshot_base<'a>(
    audience: RequestAudience,
    base_oid: &'a str,
    accepted_main_oid: Option<&str>,
    staging_repo: &FsPath,
) -> Result<Option<&'a str>, ApiError> {
    if audience != RequestAudience::Private {
        return Ok(None);
    }
    let Some(main_oid) = accepted_main_oid else {
        return Ok(None);
    };
    if !request_ref_oid_is_commit(staging_repo, main_oid)? {
        return Ok(None);
    }
    git_is_ancestor(
        staging_repo,
        base_oid,
        main_oid,
        "checking request base in accepted Git main",
    )
    .map(|in_main| in_main.then_some(base_oid))
}

pub(super) fn ensure_request_ref_descends_from_base(
    repo: &FsPath,
    base_oid: &str,
    head_oid: &str,
) -> Result<(), ApiError> {
    if git_is_ancestor(repo, base_oid, head_oid, "checking request branch ancestry")? {
        return Ok(());
    }
    Err(ApiError::conflict(
        "request branch must descend from its recorded base",
    ))
}

pub(super) fn request_revision_git_facts(
    repo: &FsPath,
    base_oid: &str,
    old_head_oid: &str,
    new_head_oid: &str,
    main_oid: Option<&str>,
) -> Result<RequestRevisionGitFacts, ApiError> {
    let contains_old_head = git_is_ancestor(
        repo,
        old_head_oid,
        new_head_oid,
        "checking whether the request push kept its old head",
    )?;
    let contained_main_oid = match main_oid {
        Some(main_oid) if request_ref_oid_is_commit(repo, main_oid)? => {
            git_merge_base(repo, main_oid, new_head_oid)?
        }
        _ => None,
    };
    let contained_main_descends_from_base = match contained_main_oid.as_deref() {
        Some(contained_main_oid) => git_is_ancestor(
            repo,
            base_oid,
            contained_main_oid,
            "checking request main against its recorded base",
        )?,
        None => false,
    };
    Ok(RequestRevisionGitFacts {
        contains_old_head,
        contained_main_oid,
        contained_main_descends_from_base,
    })
}

fn git_merge_base(repo: &FsPath, left: &str, right: &str) -> Result<Option<String>, ApiError> {
    const ACTION: &str = "reading the request head's merge base with main";
    let output = run_git_output(Some(repo), &["merge-base", left, right], ACTION)?;
    match output.status.code() {
        Some(0) => Ok(Some(
            String::from_utf8(output.stdout)
                .map_err(ApiError::bad_request)?
                .trim()
                .to_string(),
        )),
        Some(1) => Ok(None),
        _ => Err(ApiError::infrastructure_unavailable(format!(
            "{ACTION}: {}",
            crate::git::command::truncated_git_stderr(&output.stderr).trim(),
        ))),
    }
}

pub(super) fn request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(repo),
        &["cat-file", "-t", oid],
        "validating request ref commit",
    )?;
    Ok(output.status.success()
        && String::from_utf8(output.stdout)
            .map_err(ApiError::bad_request)?
            .trim()
            == "commit")
}
