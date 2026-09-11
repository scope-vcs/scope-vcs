use crate::git::import::require_git_success;
use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::{
        import::{run_git, run_git_output, validate_pushed_tree},
        projection_repo::projection_bare_repo_for_state,
    },
    state::AppState,
};
use scope_domain::{
    policy::ScopePath,
    projection::NativePublicCommit,
    projection::{ProjectionViewKey, project_graph},
    repository::Repository,
    requests::{PublicRequestPathError, PublicRequestPaths},
};
use std::{collections::BTreeSet, path::Path as FsPath};

const PUBLIC_REQUEST_BASE_REF: &str = "refs/scope/internal/public-request-base";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedPublicRequestRange {
    pub(crate) public_base_oid: String,
    pub(crate) public_parent_oids: Vec<String>,
    pub(crate) commits: Vec<NativePublicCommit>,
}

pub(super) async fn ensure_public_request_ref_is_public_safe(
    repo: &Repository,
    state: &AppState,
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<(), ApiError> {
    validate_public_request_range(repo, state, staging_repo, new_head_oid, false)
        .await
        .map(|_| ())
}

pub(crate) async fn validate_public_request_merge_range(
    repo: &Repository,
    state: &AppState,
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<ValidatedPublicRequestRange, ApiError> {
    validate_public_request_range(repo, state, staging_repo, request_head_oid, true).await
}

async fn validate_public_request_range(
    repo: &Repository,
    state: &AppState,
    staging_repo: &FsPath,
    request_head_oid: &str,
    for_merge: bool,
) -> Result<ValidatedPublicRequestRange, ApiError> {
    let (public_base_oid, public_visible_paths) =
        fetch_current_public_projection(repo, state, staging_repo).await?;
    let repo = repo.clone();
    let staging_repo = staging_repo.to_path_buf();
    let request_head_oid = request_head_oid.to_string();
    crate::git::blocking::run(move || {
        if for_merge {
            ensure_public_head_is_request_ancestor(&staging_repo, &request_head_oid)?;
        } else {
            public_request_branch_base_oid(&staging_repo, &request_head_oid)?;
        }
        let mut commits = commits_after(&staging_repo, PUBLIC_REQUEST_BASE_REF, &request_head_oid)?
            .into_iter()
            .map(|oid| public_request_commit_fact(&staging_repo, &oid, Vec::new()))
            .collect::<Result<Vec<_>, _>>()?;
        if for_merge && commits.is_empty() {
            return Err(ApiError::conflict(
                "public request contains no commits after current public main",
            ));
        }
        let public_parent_oids = validated_public_parent_oids(&staging_repo, &commits)?;
        if for_merge && !public_parent_oids.contains(&public_base_oid) {
            return Err(ApiError::conflict(
                "public main advanced; merge current public main into the request branch and push again",
            ));
        }
        let path_policy = PublicRequestPaths::new(&repo, &public_visible_paths);
        for commit in &mut commits {
            validate_pushed_tree(&staging_repo, &commit.oid)?;
            commit.changed_paths =
                ensure_public_request_commit_paths(&path_policy, &staging_repo, commit)?;
        }
        Ok(ValidatedPublicRequestRange {
            public_base_oid,
            public_parent_oids,
            commits,
        })
    })
    .await
}

fn validated_public_parent_oids(
    staging_repo: &FsPath,
    commits: &[NativePublicCommit],
) -> Result<Vec<String>, ApiError> {
    let range_oids = commits
        .iter()
        .map(|commit| commit.oid.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut public_parent_oids = BTreeSet::new();
    for commit in commits {
        let parent_oids = &commit.parent_oids;
        if parent_oids.is_empty() {
            return Err(ApiError::conflict(
                "public request history must be based on public main",
            ));
        }
        for parent_oid in parent_oids {
            if range_oids.contains(parent_oid.as_str()) {
                if !seen.contains(parent_oid.as_str()) {
                    return Err(ApiError::conflict(
                        "public request commits are not ordered ancestor-first",
                    ));
                }
            } else if git_revision_is_ancestor(staging_repo, parent_oid, PUBLIC_REQUEST_BASE_REF)? {
                public_parent_oids.insert(parent_oid.clone());
            } else {
                return Err(ApiError::conflict(
                    "public request contains a parent outside public history; rewrite the branch onto public main and push again",
                ));
            }
        }
        seen.insert(commit.oid.as_str());
    }
    if public_parent_oids.is_empty() {
        return Err(ApiError::conflict(
            "public request history must be based on public main",
        ));
    }
    Ok(public_parent_oids.into_iter().collect())
}

fn git_revision_is_ancestor(
    staging_repo: &FsPath,
    ancestor: &str,
    descendant: &str,
) -> Result<bool, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", "--is-ancestor", ancestor, descendant],
        "checking public request parent ancestry",
    )?;
    if output.status.success() {
        return Ok(true);
    }
    if output.status.code() == Some(1) {
        return Ok(false);
    }
    Err(ApiError::infrastructure_unavailable(format!(
        "checking public request parent ancestry: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}

async fn fetch_current_public_projection(
    repo: &Repository,
    state: &AppState,
    staging_repo: &FsPath,
) -> Result<(String, BTreeSet<String>), ApiError> {
    let public_projection = project_graph(
        &repo.graph,
        &repo.visibility_change_sets,
        ProjectionViewKey::Public,
    );
    if public_projection.commits.is_empty() {
        return Err(ApiError::conflict(
            "repo has no public main branch for public request",
        ));
    }
    let public_visible_paths = public_projection
        .visible_paths()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let public_repo = projection_bare_repo_for_state(
        state,
        &repo.incarnation(),
        &public_projection,
        repo.git_head.as_ref(),
        &repo.git_pack_spans,
    )
    .await?;
    let staging_repo = staging_repo.to_path_buf();
    crate::git::blocking::run(move || {
        let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{PUBLIC_REQUEST_BASE_REF}");
        run_git(
            Some(&staging_repo),
            &[
                "fetch",
                public_repo.to_string_lossy().as_ref(),
                refspec.as_str(),
            ],
            "fetching public request base",
        )?;
        let public_base_oid = git_commit_oid(&staging_repo, PUBLIC_REQUEST_BASE_REF)?;
        Ok((public_base_oid, public_visible_paths))
    })
    .await
}

fn public_request_branch_base_oid(
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<String, ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", PUBLIC_REQUEST_BASE_REF, new_head_oid],
        "checking public request branch base",
    )?;
    if !output.status.success() {
        return Err(ApiError::conflict(
            "public request branch must be based on public main",
        ));
    }
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .trim()
        .to_string())
}

fn commits_after(
    staging_repo: &FsPath,
    base: &str,
    new_head_oid: &str,
) -> Result<Vec<String>, ApiError> {
    let exclude_base = format!("^{base}");
    let output = run_git_output(
        Some(staging_repo),
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            new_head_oid,
            exclude_base.as_str(),
        ],
        "reading public request branch commits",
    )?;
    let output = require_git_success(output, "reading public request branch commits")?;
    Ok(String::from_utf8(output.stdout)
        .map_err(ApiError::bad_request)?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(ToString::to_string)
        .collect())
}

fn ensure_public_head_is_request_ancestor(
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &[
            "merge-base",
            "--is-ancestor",
            PUBLIC_REQUEST_BASE_REF,
            request_head_oid,
        ],
        "checking current public main ancestry",
    )?;
    if output.status.success() {
        return Ok(());
    }
    Err(ApiError::conflict(
        "public main advanced; merge current public main into the request branch and push again",
    ))
}

fn public_request_commit_fact(
    staging_repo: &FsPath,
    commit_oid: &str,
    changed_paths: Vec<ScopePath>,
) -> Result<NativePublicCommit, ApiError> {
    let metadata = git_text(
        staging_repo,
        &["show", "-s", "--format=%T %P", commit_oid],
        "reading public request commit identity",
    )?;
    let (tree_oid, parents) = metadata.split_once(' ').unwrap_or((&metadata, ""));
    Ok(NativePublicCommit {
        oid: commit_oid.to_string(),
        parent_oids: parents
            .split_ascii_whitespace()
            .map(ToString::to_string)
            .collect(),
        tree_oid: tree_oid.to_string(),
        changed_paths,
    })
}

fn git_commit_oid(staging_repo: &FsPath, revision: &str) -> Result<String, ApiError> {
    git_text(
        staging_repo,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
        "reading current public main",
    )
}

fn git_text(staging_repo: &FsPath, args: &[&str], context: &str) -> Result<String, ApiError> {
    crate::git::import::git_stdout_text(staging_repo, args, context)
        .map(|value| value.trim().to_string())
}

fn ensure_public_request_commit_paths(
    policy: &PublicRequestPaths<'_>,
    staging_repo: &FsPath,
    commit: &NativePublicCommit,
) -> Result<Vec<ScopePath>, ApiError> {
    let mut changed_paths = BTreeSet::new();
    for path in public_request_changed_paths(staging_repo, commit)? {
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        policy
            .ensure_editable(&scope_path)
            .map_err(|error| match error {
                PublicRequestPathError::ProtectedPath => ApiError::protected_paths(vec![path]),
                PublicRequestPathError::PrivatePath => {
                    ApiError::conflict("public request cannot change a private path")
                }
            })?;
        changed_paths.insert(scope_path);
    }
    Ok(changed_paths.into_iter().collect())
}

fn public_request_changed_paths(
    staging_repo: &FsPath,
    commit: &NativePublicCommit,
) -> Result<Vec<String>, ApiError> {
    let public_base_oid = git_commit_oid(staging_repo, PUBLIC_REQUEST_BASE_REF)?;
    let diff_base = commit
        .parent_oids
        .iter()
        .find(|parent| parent.as_str() == public_base_oid)
        .or_else(|| commit.parent_oids.first())
        .ok_or_else(|| ApiError::conflict("public request commit must have a parent"))?;
    let output = run_git_output(
        Some(staging_repo),
        &[
            "diff",
            "-r",
            "--name-only",
            "-z",
            "--no-renames",
            diff_base,
            &commit.oid,
        ],
        "reading public request commit paths",
    )?;
    let output = require_git_success(output, "reading public request commit paths")?;
    let mut changed_paths = Vec::new();
    for path in output.stdout.split(|byte| *byte == 0) {
        if path.is_empty() {
            continue;
        }
        let path = String::from_utf8(path.to_vec()).map_err(ApiError::bad_request)?;
        changed_paths.push(path);
    }
    Ok(changed_paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_tests::temp_git_repo;
    use std::{fs, path::Path};

    #[test]
    fn public_request_range_rejects_parent_outside_public_history_or_range() {
        let repo = temp_git_repo("external-parent");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let public_base = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &public_base],
            "recording public request base",
        )
        .unwrap();

        run_git(
            Some(&repo),
            &["switch", "--create", "external"],
            "creating external branch",
        )
        .unwrap();
        fs::write(repo.join("external.txt"), "external\n").unwrap();
        commit_all(&repo, "external commit");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &public_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request commit");
        let request_commit = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &[
                "merge",
                "--no-ff",
                "external",
                "-m",
                "merge external parent",
            ],
            "creating request merge with external parent",
        )
        .unwrap();
        let request_head = oid(&repo, "HEAD");

        let commits = [request_commit, request_head]
            .map(|oid| public_request_commit_fact(&repo, &oid, Vec::new()).unwrap());
        let error = validated_public_parent_oids(&repo, &commits).unwrap_err();
        assert!(
            error
                .public_message()
                .contains("parent outside public history")
        );
    }

    #[test]
    fn merge_validation_rejects_request_without_current_public_head() {
        let repo = temp_git_repo("stale-public-head");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let original_base = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &original_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request change");
        let request_head = oid(&repo, "HEAD");

        run_git(Some(&repo), &["switch", "main"], "returning to public main").unwrap();
        fs::write(repo.join("main.txt"), "advanced\n").unwrap();
        commit_all(&repo, "advance public main");
        let current_public_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &current_public_head],
            "recording advanced public request base",
        )
        .unwrap();

        assert!(ensure_public_head_is_request_ancestor(&repo, &request_head).is_err());
    }

    #[test]
    fn merge_path_validation_ignores_rules_inherited_from_current_public_main() {
        let repo = temp_git_repo("inherited-public-rules");
        fs::write(repo.join("public.txt"), "base\n").unwrap();
        commit_all(&repo, "public base");
        let original_base = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &original_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request change");

        run_git(Some(&repo), &["switch", "main"], "returning to public main").unwrap();
        fs::write(repo.join(".scope/RULES.md"), "maintainer rules\n").unwrap();
        commit_all(&repo, "update maintainer rules");
        let current_public_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", PUBLIC_REQUEST_BASE_REF, &current_public_head],
            "recording advanced public request base",
        )
        .unwrap();

        run_git(
            Some(&repo),
            &["switch", "request"],
            "returning to request branch",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &[
                "merge",
                "--no-ff",
                "main",
                "-m",
                "merge current public main",
            ],
            "merging current public main",
        )
        .unwrap();
        let merge_oid = oid(&repo, "HEAD");

        let merge = public_request_commit_fact(&repo, &merge_oid, Vec::new()).unwrap();
        let paths = public_request_changed_paths(&repo, &merge).unwrap();

        assert_eq!(paths, ["request.txt"]);

        fs::write(repo.join(".scope/RULES.md"), "request override\n").unwrap();
        run_git(
            Some(&repo),
            &["add", ".scope/RULES.md"],
            "staging request rules override",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["commit", "--amend", "--no-edit"],
            "amending merge with request rules override",
        )
        .unwrap();
        let amended_merge =
            public_request_commit_fact(&repo, &oid(&repo, "HEAD"), Vec::new()).unwrap();

        assert_eq!(
            public_request_changed_paths(&repo, &amended_merge).unwrap(),
            [".scope/RULES.md", "request.txt"]
        );
    }

    fn commit_all(repo: &Path, message: &str) {
        run_git(Some(repo), &["add", "."], "staging safety test files").unwrap();
        crate::workflow_tests::commit_all(repo, message);
    }

    fn oid(repo: &Path, revision: &str) -> String {
        git_text(repo, &["rev-parse", revision], "reading safety test oid").unwrap()
    }
}
