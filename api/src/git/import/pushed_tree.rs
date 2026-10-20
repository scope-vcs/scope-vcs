use crate::{
    config::MAX_PENDING_IMPORT_BLOB_BYTES,
    error::ApiError,
    git::command::{
        git_process_output, git_stdout_text, prepare_git_subprocess, record_git_exit,
        remaining_git_time, run_git_output, run_git_output_until, truncated_git_stderr,
    },
    runtime_budgets::RuntimeBudgets,
};
use scope_domain::content::is_supported_git_file_mode;
use scope_domain::{
    policy::ScopePath,
    repo_control::{RepoControlPath, classify_repo_control_path},
};
use scope_git::GitTreePath;
use scope_git_process::{ProcessLimits, StreamingProcessError, run_with_stdout};
use std::{
    io::{BufRead, Read},
    path::Path as FsPath,
    process::Command,
    time::Instant,
};

pub(super) fn git_tree_entries(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<Vec<GitTreeFile>, ApiError> {
    git_tree_entries_for_path(staging_repo, head_oid, None, true, None)
}

pub(super) fn git_tree_entries_under(
    staging_repo: &FsPath,
    head_oid: &str,
    path: &str,
) -> Result<Vec<GitTreeFile>, ApiError> {
    git_tree_entries_for_path(staging_repo, head_oid, Some(path), false, None)
}

fn git_tree_entries_for_path(
    staging_repo: &FsPath,
    head_oid: &str,
    path: Option<&str>,
    enforce_import_limits: bool,
    deadline: Option<Instant>,
) -> Result<Vec<GitTreeFile>, ApiError> {
    let mut args = vec!["ls-tree", "-rz", "-r", "-l", head_oid];
    if let Some(path) = path {
        args.extend(["--", path]);
    }
    let output = match deadline {
        Some(deadline) => {
            run_git_output_until(Some(staging_repo), &args, "reading pushed tree", deadline)?
        }
        None => run_git_output(Some(staging_repo), &args, "reading pushed tree")?,
    };
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed tree: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    let mut pending_files = Vec::new();
    for raw in output.stdout.split(|byte| *byte == 0) {
        if raw.is_empty() {
            continue;
        }
        let entry = std::str::from_utf8(raw).map_err(ApiError::bad_request)?;
        let Some((metadata, path)) = entry.split_once('\t') else {
            return Err(ApiError::internal_message("invalid git tree entry"));
        };
        let mut fields = metadata.split_whitespace();
        let mode = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing mode"))?;
        let kind = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing type"))?;
        let oid = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing oid"))?;
        if kind != "blob" {
            return Err(ApiError::bad_request(format!(
                "unsupported Git tree entry {path}: {kind}"
            )));
        }
        let size = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("tree entry is missing size"))?;
        let path = validate_pushed_file_path(path)?;
        if !is_supported_git_file_mode(mode) {
            return Err(ApiError::bad_request(format!(
                "unsupported Git file mode {path}: {mode}"
            )));
        }
        let blob_size = size
            .parse::<u64>()
            .map_err(|_| ApiError::internal_message("invalid Git blob size"))?;
        if enforce_import_limits && blob_size > MAX_PENDING_IMPORT_BLOB_BYTES as u64 {
            return Err(ApiError::bad_request(format!(
                "blob {path} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes"
            )));
        }
        pending_files.push(GitTreeFile {
            path,
            mode: mode.to_string(),
            oid: oid.to_string(),
            size_bytes: blob_size,
        });
    }

    Ok(pending_files)
}

pub(crate) fn git_changed_tree_entries(
    staging_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
) -> Result<Vec<(ScopePath, Option<GitTreeFile>)>, ApiError> {
    git_changed_tree_entries_until(
        staging_repo,
        base_oid,
        head_oid,
        Instant::now() + RuntimeBudgets::default_git_command_timeout(),
    )
}

pub(crate) fn git_changed_tree_entries_until(
    staging_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
    deadline: Instant,
) -> Result<Vec<(ScopePath, Option<GitTreeFile>)>, ApiError> {
    let Some(base_oid) = base_oid else {
        return git_tree_entries_for_path(staging_repo, head_oid, None, true, Some(deadline))?
            .into_iter()
            .map(|entry| Ok((entry.path.to_scope_path(), Some(entry))))
            .collect();
    };
    let output = run_git_output_until(
        Some(staging_repo),
        &[
            "diff-tree",
            "--no-commit-id",
            "--raw",
            "-r",
            "-z",
            "--no-renames",
            base_oid,
            head_oid,
        ],
        "reading pushed Git delta",
        deadline,
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed Git delta: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let mut pending = raw_delta_entries(&output.stdout)?;
    fill_blob_sizes(staging_repo, &mut pending, deadline)?;
    Ok(pending)
}

fn raw_delta_entries(stdout: &[u8]) -> Result<Vec<(ScopePath, Option<GitTreeFile>)>, ApiError> {
    let mut fields = stdout.split(|byte| *byte == 0);
    let mut pending = Vec::new();
    while let Some(header) = fields.next() {
        if header.is_empty() {
            continue;
        }
        let path = fields
            .next()
            .ok_or_else(|| ApiError::internal_message("Git delta is missing a path"))?;
        pending.push(raw_delta_record(header, path)?);
    }
    Ok(pending)
}

fn raw_delta_record(
    header: &[u8],
    path: &[u8],
) -> Result<(ScopePath, Option<GitTreeFile>), ApiError> {
    let header = std::str::from_utf8(header).map_err(ApiError::bad_request)?;
    let path = std::str::from_utf8(path).map_err(ApiError::bad_request)?;
    let path = validate_pushed_file_path(path)?;
    let values = header.split_whitespace().collect::<Vec<_>>();
    if values.len() != 5 || !values[0].starts_with(':') {
        return Err(ApiError::internal_message("invalid Git delta record"));
    }
    let new_mode = values[1];
    let new_oid = values[3];
    let status = values[4];
    let scope_path = path.to_scope_path();
    if status == "D" {
        return Ok((scope_path, None));
    }
    if !is_supported_git_file_mode(new_mode) {
        return Err(ApiError::bad_request(format!(
            "unsupported Git file mode {path}: {new_mode}"
        )));
    }
    Ok((
        scope_path,
        Some(GitTreeFile {
            path,
            mode: new_mode.to_string(),
            oid: new_oid.to_string(),
            size_bytes: 0,
        }),
    ))
}

fn unique_added_blobs(
    stdout: impl Read,
) -> Result<Vec<(ScopePath, Option<GitTreeFile>)>, ApiError> {
    let mut reader = std::io::BufReader::new(stdout);
    let mut seen = std::collections::BTreeSet::new();
    let mut added = Vec::new();
    let mut header = Vec::new();
    let mut path = Vec::new();
    loop {
        header.clear();
        if reader
            .read_until(0, &mut header)
            .map_err(ApiError::internal)?
            == 0
        {
            return Ok(added);
        }
        path.clear();
        reader
            .read_until(0, &mut path)
            .map_err(ApiError::internal)?;
        let (Some(header), Some(path)) = (header.strip_suffix(&[0]), path.strip_suffix(&[0]))
        else {
            return Err(ApiError::internal_message("Git delta is missing a path"));
        };
        let record = raw_delta_record(header, path)?;
        if record
            .1
            .as_ref()
            .is_some_and(|entry| seen.insert(entry.oid.clone()))
        {
            added.push(record);
        }
    }
}

fn fill_blob_sizes(
    staging_repo: &FsPath,
    pending: &mut [(ScopePath, Option<GitTreeFile>)],
    deadline: Instant,
) -> Result<(), ApiError> {
    let requested_oids = pending
        .iter()
        .filter_map(|(_, entry)| entry.as_ref().map(|entry| entry.oid.as_str()))
        .map(|oid| format!("{oid}\n"))
        .collect::<String>();
    if requested_oids.is_empty() {
        return Ok(());
    }
    let output = git_process_output(
        Command::new("git").current_dir(staging_repo).args([
            "cat-file",
            "--batch-check=%(objectname) %(objecttype) %(objectsize)",
        ]),
        Some(requested_oids.into_bytes()),
        ProcessLimits::new(remaining_git_time(deadline)?),
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed blob sizes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let size_output = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
    let mut sizes = size_output.lines().map(|line| {
        let values = line.split_whitespace().collect::<Vec<_>>();
        if values.len() != 3 || values[1] != "blob" {
            return Err(ApiError::bad_request("pushed path is not a Git blob"));
        }
        values[2]
            .parse::<u64>()
            .map_err(|_| ApiError::internal_message("invalid Git blob size"))
    });
    for entry in pending.iter_mut().filter_map(|(_, entry)| entry.as_mut()) {
        entry.size_bytes = sizes
            .next()
            .ok_or_else(|| ApiError::internal_message("missing Git blob size"))??;
        if entry.size_bytes > MAX_PENDING_IMPORT_BLOB_BYTES as u64 {
            return Err(ApiError::bad_request(format!(
                "blob {} is larger than {MAX_PENDING_IMPORT_BLOB_BYTES} bytes",
                entry.path
            )));
        }
    }
    Ok(())
}

pub(crate) fn validate_pushed_tree(staging_repo: &FsPath, head_oid: &str) -> Result<(), ApiError> {
    git_tree_entries(staging_repo, head_oid)?;
    Ok(())
}

pub(crate) fn validate_pushed_commit_range(
    staging_repo: &FsPath,
    base_oid: Option<&str>,
    head_oid: &str,
) -> Result<(), ApiError> {
    let deadline = Instant::now() + RuntimeBudgets::default_git_command_timeout();
    let mut args = vec!["rev-list", "--reverse", head_oid];
    let excluded_base = base_oid.map(|oid| format!("^{oid}"));
    if let Some(excluded_base) = excluded_base.as_deref() {
        args.push(excluded_base);
    }
    let commits = git_stdout_text(staging_repo, &args, "reading pushed commit range")?;
    if commits.trim().is_empty() {
        return Ok(());
    }
    let mut command = Command::new("git");
    command.current_dir(staging_repo).args([
        "diff-tree",
        "--stdin",
        "--root",
        "--no-commit-id",
        "--raw",
        "-r",
        "-z",
        "--no-renames",
        "--diff-merges=first-parent",
    ]);
    let git_span = prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let output = run_with_stdout(
        &mut command,
        Some(commits.into_bytes()),
        ProcessLimits::new(remaining_git_time(deadline)?),
        "reading pushed commit changes",
        |stdout, _| unique_added_blobs(stdout),
    )
    .map_err(|error| match error {
        StreamingProcessError::Process(error) => {
            ApiError::infrastructure_unavailable(error.to_string())
        }
        StreamingProcessError::Consumer(error) => error,
    })?;
    record_git_exit(&git_span, output.status);
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading pushed commit changes: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let mut added = output.value;
    fill_blob_sizes(staging_repo, &mut added, deadline)
}

#[derive(Debug)]
pub(crate) struct GitTreeFile {
    pub(crate) path: GitTreePath,
    pub(crate) mode: String,
    pub(crate) oid: String,
    pub(crate) size_bytes: u64,
}

pub(crate) fn validate_pushed_file_path(path: &str) -> Result<GitTreePath, ApiError> {
    let path = GitTreePath::parse(path).map_err(ApiError::bad_request)?;
    let scope_path = path.to_scope_path();
    if matches!(
        classify_repo_control_path(&scope_path),
        Some(RepoControlPath::Forbidden)
    ) {
        return Err(ApiError::bad_request(format!(
            "Scope control path {:?} is not a supported tracked control file",
            path.as_str()
        )));
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::command::run_git;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn workflow_tree_listing_is_scoped_to_the_runs_directory() {
        let root = std::env::temp_dir().join(format!(
            "scope-workflow-tree-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join(".scope/runs")).unwrap();
        run_git(Some(&root), &["init", "-q"], "initializing test repository").unwrap();
        fs::write(root.join("README.md"), "unrelated\n").unwrap();
        fs::write(root.join(".scope/runs/test.yml"), "name: Test\n").unwrap();
        run_git(Some(&root), &["add", "."], "staging test repository").unwrap();
        run_git(
            Some(&root),
            &[
                "-c",
                "user.name=Scope Tests",
                "-c",
                "user.email=scope@example.com",
                "commit",
                "-qm",
                "test",
            ],
            "committing test repository",
        )
        .unwrap();
        let head = git_stdout_text(&root, &["rev-parse", "HEAD"], "reading test head")
            .unwrap()
            .trim()
            .to_string();

        let entries = git_tree_entries_under(&root, &head, ".scope/runs").unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path.as_str(), ".scope/runs/test.yml");

        let invalid = git_tree_entries_under(&root, "invalid-object", ".scope/runs").unwrap_err();
        assert_eq!(
            invalid.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        let subtree =
            git_stdout_text(&root, &["rev-parse", "HEAD:.scope/runs"], "reading subtree").unwrap();
        let subtree = subtree.trim();
        fs::remove_file(
            root.join(".git/objects")
                .join(&subtree[..2])
                .join(&subtree[2..]),
        )
        .unwrap();
        let missing = git_tree_entries_under(&root, &head, ".scope/runs").unwrap_err();
        assert_eq!(
            missing.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(git_tree_entries(&root, &head).is_err());
        fs::remove_dir_all(root).unwrap();
    }
}
