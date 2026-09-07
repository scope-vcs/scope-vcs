use super::*;

pub fn changed_paths_since_scope_base_at_commit(
    repo: &GitRepo,
    base_oid_or_ref: Option<&str>,
    commit_oid: &str,
) -> anyhow::Result<Vec<GitChangedPath>> {
    match base_oid_or_ref {
        Some(base) => {
            let output = git_output_in_repo(
                repo,
                &[
                    "diff",
                    "--name-status",
                    "-z",
                    "--find-renames",
                    &format!("{base}..{commit_oid}"),
                    "--",
                ],
            )?;
            if !output.status.success() {
                bail!("inspect committed changes for Scope push review failed");
            }

            parse_name_status(&output.stdout)
        }
        None => {
            let output = git_output_in_repo(repo, &["ls-tree", "-rz", "--name-only", commit_oid])?;
            if !output.status.success() {
                bail!("inspect committed files for Scope first push review failed");
            }

            parse_tree_paths_as_added(&output.stdout)
        }
    }
}

pub fn request_side_changed_file_paths(
    repo: &GitRepo,
    recorded_base_oid: &str,
    current_main_oid: &str,
    request_head_oid: &str,
) -> anyhow::Result<Vec<String>> {
    ensure_commit_exists(repo, recorded_base_oid, "recorded request base")?;
    ensure_commit_exists(repo, current_main_oid, "current main")?;
    ensure_commit_exists(repo, request_head_oid, "request head")?;

    let merge_base_output = git_output_in_repo(
        repo,
        &["merge-base", "--all", current_main_oid, request_head_oid],
    )?;
    if !merge_base_output.status.success() {
        if merge_base_output.status.code() == Some(1) {
            bail!(
                "current main and request head have unrelated Git histories; Scope requests must descend from the repository's Scope main. Run `scope request start <name>` and replay the GitHub branch changes onto that request branch before `scope request push`"
            );
        }
        bail!("find the request branch merge base failed");
    }
    let merge_base_oids = String::from_utf8_lossy(&merge_base_output.stdout)
        .lines()
        .map(str::trim)
        .filter(|oid| !oid.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    let merge_base_oid = match merge_base_oids.as_slice() {
        [] => bail!("Git did not return a request branch merge base"),
        [merge_base_oid] => merge_base_oid,
        _ => bail!("current main and request head have multiple Git merge bases"),
    };
    ensure_recorded_base_ancestor_of_merge_base(repo, recorded_base_oid, merge_base_oid)?;

    let request_output = git_output_in_repo(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            merge_base_oid,
            request_head_oid,
        ],
    )?;
    if !request_output.status.success() {
        bail!("inspect request-side committed paths failed");
    }

    let merge_output = git_output_in_repo(
        repo,
        &[
            "merge-tree",
            "--write-tree",
            "--no-messages",
            "--name-only",
            "-z",
            current_main_oid,
            request_head_oid,
        ],
    )?;
    if !merge_output.status.success() && merge_output.status.code() != Some(1) {
        bail!("compute the request merge result failed");
    }
    let merge_tree_separator = merge_output
        .stdout
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| anyhow::anyhow!("Git did not return a request merge result tree"))?;
    let merge_tree_oid =
        String::from_utf8_lossy(&merge_output.stdout[..merge_tree_separator]).to_string();
    if merge_tree_oid.is_empty() {
        bail!("Git did not return a request merge result tree");
    }
    let conflict_paths = parse_nul_paths(&merge_output.stdout[merge_tree_separator + 1..])?;

    let merge_result_output = git_output_in_repo(
        repo,
        &[
            "diff",
            "--name-only",
            "-z",
            "--no-renames",
            current_main_oid,
            &merge_tree_oid,
        ],
    )?;
    if !merge_result_output.status.success() {
        bail!("inspect request merge result paths failed");
    }
    let mut paths = parse_nul_paths(&request_output.stdout)?
        .into_iter()
        .collect::<BTreeSet<_>>();
    paths.extend(parse_nul_paths(&merge_result_output.stdout)?);
    paths.extend(conflict_paths);
    Ok(paths.into_iter().collect())
}

fn ensure_recorded_base_ancestor_of_merge_base(
    repo: &GitRepo,
    recorded_base_oid: &str,
    merge_base_oid: &str,
) -> anyhow::Result<()> {
    let output = git_output_in_repo(
        repo,
        &[
            "merge-base",
            "--is-ancestor",
            recorded_base_oid,
            merge_base_oid,
        ],
    )?;
    if output.status.success() {
        return Ok(());
    }
    if output.status.code() == Some(1) {
        bail!("request branch merge base does not descend from the recorded request base");
    }
    bail!("validate request branch ancestry failed")
}

fn ensure_commit_exists(repo: &GitRepo, revision: &str, label: &str) -> anyhow::Result<()> {
    let commit = format!("{revision}^{{commit}}");
    let output = git_output_in_repo(repo, &["rev-parse", "--verify", "--quiet", &commit])?;
    if !output.status.success() {
        return Err(crate::error::CliError::usage(format!(
            "{label} commit is missing from the local Git repository"
        ))
        .into());
    }
    Ok(())
}

pub fn worktree_file_paths(repo: &GitRepo) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(
        repo,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )?;
    if !output.status.success() {
        bail!("inspect Git worktree files failed");
    }

    let deleted_output = git_output_in_repo(repo, &["ls-files", "-z", "--deleted"])?;
    if !deleted_output.status.success() {
        bail!("inspect deleted Git worktree files failed");
    }

    Ok(exclude_deleted_paths(
        parse_nul_paths(&output.stdout)?,
        parse_nul_paths(&deleted_output.stdout)?,
    ))
}

pub fn committed_file_paths_at_commit(
    repo: &GitRepo,
    commit_oid: &str,
) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(repo, &["ls-tree", "-rz", "--name-only", commit_oid])?;
    if !output.status.success() {
        bail!("inspect committed files for Scope review failed");
    }

    parse_nul_paths(&output.stdout)
}

fn parse_name_status(output: &[u8]) -> anyhow::Result<Vec<GitChangedPath>> {
    let mut fields = output
        .strip_suffix(&[0])
        .unwrap_or(output)
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty());
    let mut changes = Vec::new();
    while let Some(status) = fields.next() {
        let status = std::str::from_utf8(status).context("Git change status is not UTF-8")?;
        let path = fields.next().context("Git change is missing its path")?;
        let (previous_path, path) = if status.starts_with(['R', 'C']) {
            (
                Some(decode_path(path)?),
                fields
                    .next()
                    .context("Git rename is missing its destination")?,
            )
        } else {
            (None, path)
        };
        changes.push(GitChangedPath {
            status: status.to_owned(),
            path: decode_path(path)?,
            previous_path,
        });
    }
    Ok(changes)
}

fn parse_tree_paths_as_added(output: &[u8]) -> anyhow::Result<Vec<GitChangedPath>> {
    Ok(parse_nul_paths(output)?
        .into_iter()
        .map(|path| GitChangedPath {
            status: "A".to_string(),
            path,
            previous_path: None,
        })
        .collect())
}

fn decode_path(path: &[u8]) -> anyhow::Result<String> {
    String::from_utf8(path.to_vec()).map_err(|_| crate::error::CliError::usage(
        "Git contains a filename that is not UTF-8; rename it to a UTF-8 filename before using Scope"
    ).into())
}

fn parse_nul_paths(output: &[u8]) -> anyhow::Result<Vec<String>> {
    output
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(decode_path)
        .collect()
}

fn exclude_deleted_paths(paths: Vec<String>, deleted_paths: Vec<String>) -> Vec<String> {
    let deleted_paths = deleted_paths.into_iter().collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .filter(|path| !deleted_paths.contains(path))
        .collect()
}
