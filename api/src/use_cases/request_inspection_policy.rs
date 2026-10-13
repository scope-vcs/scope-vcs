use crate::{
    error::ApiError, git::blocking::block_on, runtime_budgets::RuntimeBudgets, state::AppState,
};
use scope_domain::{
    policy::{Policy, ScopePath},
    requests::RequestRevision,
    views::ViewId,
};
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use scope_postgres::db::RepositoryReadPolicy;
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    path::Path,
    process::Command,
};

pub(crate) const MAX_REQUEST_INSPECTION_PATHS: usize = 10_000;
const MAX_INSPECTED_PATH_BYTES: usize = 64 * 1024;

pub(crate) fn resolve_revision(
    state: &AppState,
    repo: &RepositoryReadPolicy,
    reader: &ViewId,
    raw_repo: &Path,
    revision: &RequestRevision,
    selected_commit: Option<&str>,
    commit_limit: usize,
) -> Result<Policy, ApiError> {
    let mut paths = BTreeSet::new();
    let comparison = scope_domain::requests::RequestComparison::push(revision, reader.clone());
    let mut commits = super::request_revision_inspection::request_revision_commit_oids(
        raw_repo,
        &comparison,
        commit_limit,
    )?;
    if commits.len() > commit_limit {
        commits.drain(..commits.len() - commit_limit);
    }
    if let Some(commit) = selected_commit {
        commits.push(commit.to_string());
    }
    let mut commits = commits.join("\n").into_bytes();
    commits.push(b'\n');
    collect_paths(
        raw_repo,
        &["--stdin", "--diff-merges=first-parent"],
        Some(commits),
        &mut paths,
    )?;
    let comparison = scope_domain::requests::RequestComparison::request(revision, reader.clone());
    let base =
        super::request_revision_inspection::request_comparison_base_oid(raw_repo, &comparison)?;
    collect_paths(
        raw_repo,
        &[
            base.as_deref().expect("request base exists"),
            &revision.new_head_oid,
        ],
        None,
        &mut paths,
    )?;
    collect_paths(
        raw_repo,
        &[&revision.old_head_oid, &revision.new_head_oid],
        None,
        &mut paths,
    )?;
    resolve_paths(state, repo, reader, paths)
}

pub(crate) fn resolve_commit(
    state: &AppState,
    repo: &RepositoryReadPolicy,
    reader: &ViewId,
    raw_repo: &Path,
    commit: &str,
) -> Result<Policy, ApiError> {
    resolve_commits(state, repo, reader, raw_repo, &[commit.to_string()])
}

pub(crate) fn resolve_commits(
    state: &AppState,
    repo: &RepositoryReadPolicy,
    reader: &ViewId,
    raw_repo: &Path,
    commits: &[String],
) -> Result<Policy, ApiError> {
    let mut paths = BTreeSet::new();
    let mut input = commits.join("\n").into_bytes();
    input.push(b'\n');
    collect_paths(
        raw_repo,
        &["--stdin", "--diff-merges=first-parent"],
        Some(input),
        &mut paths,
    )?;
    resolve_paths(state, repo, reader, paths)
}

pub(crate) fn resolve_paths(
    state: &AppState,
    repo: &RepositoryReadPolicy,
    reader: &ViewId,
    paths: BTreeSet<ScopePath>,
) -> Result<Policy, ApiError> {
    let paths = paths.into_iter().collect::<Vec<_>>();
    let history = block_on(state.metadata.repositories().repository_path_history(
        &repo.context.incarnation(),
        repo.context.record.content_version,
        &paths,
    ))?;
    Ok(repo
        .policy
        .for_request_paths(&repo.repo_config, &history, reader, &paths))
}

fn collect_paths(
    raw_repo: &Path,
    args: &[&str],
    input: Option<Vec<u8>>,
    paths: &mut BTreeSet<ScopePath>,
) -> Result<(), ApiError> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(raw_repo)
        .args([
            "diff-tree",
            "--root",
            "--no-commit-id",
            "-r",
            "--name-only",
            "-z",
            "--no-renames",
        ])
        .args(args);
    let span = crate::git::command::prepare_git_subprocess(&mut command)?;
    let _entered = span.enter();
    let mut collected = paths.clone();
    let output = run_with_stdout(
        &mut command,
        input,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "reading bounded request inspection paths",
        move |stdout, _| {
            let mut reader = BufReader::new(stdout);
            loop {
                let mut bytes = Vec::new();
                let mut oversized = false;
                loop {
                    let available = reader.fill_buf().map_err(ApiError::internal)?;
                    if available.is_empty() {
                        return Ok::<_, ApiError>(collected);
                    }
                    let consumed = available
                        .iter()
                        .position(|byte| *byte == 0)
                        .map_or(available.len(), |index| index + 1);
                    let terminated = available[consumed - 1] == 0;
                    let content = &available[..consumed - usize::from(terminated)];
                    let remaining = MAX_INSPECTED_PATH_BYTES.saturating_sub(bytes.len());
                    bytes.extend_from_slice(&content[..content.len().min(remaining)]);
                    oversized |= content.len() > remaining;
                    reader.consume(consumed);
                    if terminated {
                        break;
                    }
                }
                if oversized {
                    return Err(ApiError::payload_too_large(
                        "request inspection path exceeds the 64 KiB limit",
                    ));
                }
                if !bytes.is_empty() {
                    let path = std::str::from_utf8(&bytes).map_err(ApiError::bad_request)?;
                    let path =
                        ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
                    if !collected.contains(&path) && collected.len() == MAX_REQUEST_INSPECTION_PATHS
                    {
                        return Err(ApiError::payload_too_large(
                            "request inspection exceeds the 10,000 path limit",
                        ));
                    }
                    collected.insert(path);
                }
            }
        },
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    crate::git::command::record_git_exit(&span, output.status);
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(
            "reading request inspection paths failed",
        ));
    }
    *paths = output.value;
    Ok(())
}
