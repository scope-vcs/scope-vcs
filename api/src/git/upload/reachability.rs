use crate::{
    error::ApiError,
    git::command::{
        git_command_output, prepare_git_subprocess, record_git_exit, truncated_git_stderr,
    },
    runtime_budgets::RuntimeBudgets,
};
use scope_git_process::{ProcessLimits, run_with_stdout};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{BufRead, BufReader},
    path::Path as FsPath,
    process::Command,
};

const COMMIT: &[u8] = b"commit";

pub(super) fn unreachable_from_refs(
    repo: &FsPath,
    candidates: &BTreeSet<Vec<u8>>,
) -> Result<BTreeSet<Vec<u8>>, ApiError> {
    let kinds = object_kinds(repo, candidates)?;
    let (present, mut unreachable): (BTreeSet<Vec<u8>>, BTreeSet<Vec<u8>>) = candidates
        .iter()
        .cloned()
        .partition(|oid| kinds.contains_key(oid));
    if present.is_empty() {
        return Ok(unreachable);
    }
    let bounded = present
        .iter()
        .all(|oid| kinds[oid] == COMMIT)
        .then(|| commits_outside_refs(repo, &present).ok())
        .flatten();
    match bounded {
        Some(outside) => unreachable.extend(outside),
        None => {
            let reachable = objects_reachable_from_refs(repo, present.clone())?;
            unreachable.extend(present.difference(&reachable).cloned());
        }
    }
    Ok(unreachable)
}

fn object_kinds(
    repo: &FsPath,
    candidates: &BTreeSet<Vec<u8>>,
) -> Result<BTreeMap<Vec<u8>, Vec<u8>>, ApiError> {
    let output = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo)
            .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"]),
        Some(&oid_lines(candidates)),
    )?;
    Ok(output
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let (oid, kind) = line.split_at(line.iter().position(|byte| *byte == b' ')?);
            let kind = &kind[1..];
            (kind != b"missing").then(|| (oid.to_vec(), kind.to_vec()))
        })
        .collect())
}

fn oid_lines(oids: &BTreeSet<Vec<u8>>) -> Vec<u8> {
    oids.iter().fold(Vec::new(), |mut lines, oid| {
        lines.extend_from_slice(oid);
        lines.push(b'\n');
        lines
    })
}

fn commits_outside_refs(
    repo: &FsPath,
    commits: &BTreeSet<Vec<u8>>,
) -> Result<BTreeSet<Vec<u8>>, ApiError> {
    let output = git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(repo)
            .args(["rev-list", "--stdin", "--not", "--all"]),
        Some(&oid_lines(commits)),
    )?;
    Ok(output
        .split(|byte| *byte == b'\n')
        .filter(|oid| commits.contains(*oid))
        .map(<[u8]>::to_vec)
        .collect())
}

fn objects_reachable_from_refs(
    repo: &FsPath,
    mut candidates: BTreeSet<Vec<u8>>,
) -> Result<BTreeSet<Vec<u8>>, ApiError> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo)
        .args(["rev-list", "--objects", "--all"]);
    let git_span = prepare_git_subprocess(&mut command)?;
    let _entered = git_span.enter();
    let all = candidates.clone();
    let walked = run_with_stdout(
        &mut command,
        None,
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout()),
        "checking Git upload-pack object reachability",
        move |stdout, _cancellation| {
            for line in BufReader::new(stdout).split(b'\n') {
                let line = line?;
                let oid = line.split(|byte| *byte == b' ').next().unwrap_or_default();
                candidates.remove(oid);
            }
            Ok::<_, std::io::Error>(candidates)
        },
    )
    .map_err(|error| ApiError::infrastructure_unavailable(error.to_string()))?;
    record_git_exit(&git_span, walked.status);
    if !walked.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "checking Git upload-pack object reachability: {}",
            truncated_git_stderr(&walked.stderr)
        )));
    }
    Ok(all.difference(&walked.value).cloned().collect())
}

#[cfg(test)]
mod tests;
