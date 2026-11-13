use crate::{
    error::ApiError,
    git::{
        command::{git_is_ancestor, run_git_output},
        reader::{GitReader, MergeBases},
    },
};
use scope_domain::requests::RequestRevisionGitFacts;
use std::{collections::BTreeSet, path::Path as FsPath};

pub(super) fn ensure_request_ref_descends_from_bases(
    repo: &FsPath,
    main_ancestor_oid: &str,
    review_base_oid: &str,
    head_oid: &str,
) -> Result<(), ApiError> {
    ensure_request_ref_descends_from_base(repo, main_ancestor_oid, head_oid)?;
    if review_base_oid == main_ancestor_oid {
        return Ok(());
    }
    ensure_request_ref_descends_from_base(repo, review_base_oid, head_oid)
}

fn ensure_request_ref_descends_from_base(
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

pub(crate) fn request_revision_git_facts(
    repo: &FsPath,
    base_oid: &str,
    old_head_oid: &str,
    new_head_oid: &str,
    main_oid: Option<&str>,
) -> Result<RequestRevisionGitFacts, ApiError> {
    let mut present = BTreeSet::new();
    if let Some(main_oid) = main_oid
        && request_ref_oid_is_commit(repo, main_oid)?
    {
        present.insert(main_oid.to_string());
    }
    revision_git_facts(
        repo,
        &present,
        base_oid,
        old_head_oid,
        new_head_oid,
        main_oid,
    )
}

pub(super) fn revision_git_facts(
    repo: &FsPath,
    present_commits: &BTreeSet<String>,
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
        Some(main_oid) if present_commits.contains(main_oid) => {
            git_merge_base(repo, main_oid, new_head_oid)?
        }
        _ => None,
    };
    let contained_main_descends_from_base = match contained_main_oid.as_deref() {
        Some(contained_main_oid) if contained_main_oid == base_oid => true,
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
    if let Some(reader) = GitReader::open(repo) {
        match reader.merge_bases(left, right, ACTION)? {
            Some(MergeBases::Unrelated) => return Ok(None),
            Some(MergeBases::Unique(base)) => return Ok(Some(base)),
            Some(MergeBases::Several) | None => {}
        }
    }
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
    if let Some(is_commit) = GitReader::open(repo).and_then(|reader| reader.is_commit(oid)) {
        return Ok(is_commit);
    }
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
