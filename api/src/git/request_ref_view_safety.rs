use crate::{
    error::ApiError,
    git::{
        command::{
            git_is_ancestor, git_stdout_text, run_git, run_git_output, successful_git_output,
        },
        import::validate_pushed_tree,
        repository_git::RepositoryGit,
    },
    state::AppState,
};
use scope_domain::{
    policy::ScopePath,
    projection::NativeRequestCommit,
    repo_config::RepoConfig,
    requests::{PathHistory, RequestViewPathError, RequestViewPaths},
    views::{ViewId, Views},
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{collections::BTreeSet, path::Path as FsPath};

const VIEW_REQUEST_BASE_REF: &str = "refs/scope/internal/view-request-base";
const VIEW_MAIN_MOVED_ERROR: &str =
    "Request view main moved. Rebase onto it or merge it, then run scope request push.";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ValidatedViewRequestRange {
    pub(crate) base_oid: String,
    pub(crate) parent_oids: Vec<String>,
    pub(crate) commits: Vec<NativeRequestCommit>,
}

#[derive(Clone, Copy)]
pub(crate) struct RequestView<'a> {
    git: &'a RepositoryGit,
    views: &'a Views,
    view: &'a ViewId,
}

impl<'a> RequestView<'a> {
    pub(crate) fn new(git: &'a RepositoryGit, views: &'a Views, view: &'a ViewId) -> Self {
        Self { git, views, view }
    }

    fn name(&self) -> &'a str {
        self.views.display_name(self.view)
    }
}

pub(crate) async fn ensure_request_ref_is_view_safe(
    request_view: RequestView<'_>,
    repo_config: &RepoConfig,
    state: &AppState,
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<String, ApiError> {
    let (view_main_oid, visible_paths) =
        fetch_current_view_projection(request_view, state, staging_repo).await?;
    ensure_request_branch_is_based_on_view_main(request_view, staging_repo, new_head_oid)?;
    let commit_oids = commits_after(staging_repo, VIEW_REQUEST_BASE_REF, new_head_oid)?;
    validated_view_parent_oids(request_view.name(), staging_repo, &commit_oids)?;
    let commits = validated_changed_paths_by_commit(staging_repo, commit_oids)?;
    let history = changed_path_history(request_view.git, state, &commits).await?;
    let policy = request_view_paths(request_view, repo_config, &visible_paths, &history);
    for (_, paths) in &commits {
        ensure_view_request_commit_paths(&policy, paths)?;
    }
    Ok(view_main_oid)
}

pub(crate) async fn validate_view_request_merge_range(
    request_view: RequestView<'_>,
    repo_config: &RepoConfig,
    state: &AppState,
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<ValidatedViewRequestRange, ApiError> {
    let (base_oid, visible_paths) =
        fetch_current_view_projection(request_view, state, staging_repo).await?;
    ensure_view_head_is_request_ancestor(staging_repo, request_head_oid)?;
    let commit_oids = commits_after(staging_repo, VIEW_REQUEST_BASE_REF, request_head_oid)?;
    let view_name = request_view.name();
    if commit_oids.is_empty() {
        return Err(ApiError::conflict(format!(
            "{view_name} request contains no commits after current {view_name} main"
        )));
    }
    let parent_oids = validated_view_parent_oids(view_name, staging_repo, &commit_oids)?;
    if !parent_oids.contains(&base_oid) {
        return Err(ApiError::conflict(VIEW_MAIN_MOVED_ERROR));
    }

    let changed = validated_changed_paths_by_commit(staging_repo, commit_oids)?;
    let history = changed_path_history(request_view.git, state, &changed).await?;
    let policy = request_view_paths(request_view, repo_config, &visible_paths, &history);
    let mut commits = Vec::with_capacity(changed.len());
    for (commit_oid, paths) in &changed {
        let changed_paths = ensure_view_request_commit_paths(&policy, paths)?;
        commits.push(native_request_commit(
            staging_repo,
            commit_oid,
            changed_paths,
        )?);
    }

    Ok(ValidatedViewRequestRange {
        base_oid,
        parent_oids,
        commits,
    })
}

pub(crate) async fn view_contribution_base(
    request_view: RequestView<'_>,
    state: &AppState,
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<String, ApiError> {
    fetch_current_view_projection(request_view, state, staging_repo).await?;
    git_stdout_text(
        staging_repo,
        &["merge-base", VIEW_REQUEST_BASE_REF, request_head_oid],
        "finding the request view contribution base",
    )
    .map(|oid| oid.trim().to_string())
}

fn validated_view_parent_oids(
    view_name: &str,
    staging_repo: &FsPath,
    commit_oids: &[String],
) -> Result<Vec<String>, ApiError> {
    let based_on_view_main = || {
        ApiError::conflict(format!(
            "{view_name} request history must be based on {view_name} main"
        ))
    };
    let range_oids = commit_oids
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    let mut view_parent_oids = BTreeSet::new();
    for commit_oid in commit_oids {
        let parent_oids = git_stdout_text(
            staging_repo,
            &["show", "-s", "--format=%P", commit_oid],
            "reading request commit parents",
        )?
        .split_ascii_whitespace()
        .map(ToString::to_string)
        .collect::<Vec<_>>();
        if parent_oids.is_empty() {
            return Err(based_on_view_main());
        }
        for parent_oid in parent_oids {
            if range_oids.contains(parent_oid.as_str()) {
                if !seen.contains(parent_oid.as_str()) {
                    return Err(ApiError::conflict(format!(
                        "{view_name} request commits are not ordered ancestor-first"
                    )));
                }
            } else if git_is_ancestor(
                staging_repo,
                &parent_oid,
                VIEW_REQUEST_BASE_REF,
                "checking request parent ancestry",
            )? {
                view_parent_oids.insert(parent_oid);
            } else {
                return Err(ApiError::conflict(format!(
                    "{view_name} request contains a parent outside {view_name} history"
                )));
            }
        }
        seen.insert(commit_oid.as_str());
    }
    if view_parent_oids.is_empty() {
        return Err(based_on_view_main());
    }
    Ok(view_parent_oids.into_iter().collect())
}

async fn fetch_current_view_projection(
    request_view: RequestView<'_>,
    state: &AppState,
    staging_repo: &FsPath,
) -> Result<(String, BTreeSet<String>), ApiError> {
    let views = request_view.views;
    if views.get(request_view.view).is_none() {
        return Err(ApiError::not_found(format!(
            "view {} not found",
            request_view.view
        )));
    }
    let git = request_view.git;
    let Some(head_oid) = git.view_head(state, request_view.view).await? else {
        return Err(ApiError::conflict(format!(
            "repo has no {} main branch for this request",
            request_view.name()
        )));
    };
    let view_repo = git
        .view_repo_at(state, views, request_view.view, Some(&head_oid))
        .await?;
    let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{VIEW_REQUEST_BASE_REF}");
    run_git(
        Some(staging_repo),
        &[
            "fetch",
            view_repo.to_string_lossy().as_ref(),
            refspec.as_str(),
        ],
        "fetching the request view base",
    )?;
    let base_oid = git_commit_oid(staging_repo, VIEW_REQUEST_BASE_REF)?;
    Ok((base_oid, view_tree_paths(staging_repo)?))
}

fn view_tree_paths(staging_repo: &FsPath) -> Result<BTreeSet<String>, ApiError> {
    let action = "listing request view base paths";
    let output = successful_git_output(
        run_git_output(
            Some(staging_repo),
            &["ls-tree", "-r", "--name-only", "-z", VIEW_REQUEST_BASE_REF],
            action,
        )?,
        action,
    )?;
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| {
            String::from_utf8(path.to_vec())
                .map(|path| format!("/{path}"))
                .map_err(ApiError::internal)
        })
        .collect()
}

fn ensure_request_branch_is_based_on_view_main(
    request_view: RequestView<'_>,
    staging_repo: &FsPath,
    new_head_oid: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(staging_repo),
        &["merge-base", VIEW_REQUEST_BASE_REF, new_head_oid],
        "checking request branch base",
    )?;
    if output.status.success() {
        return Ok(());
    }
    let view_name = request_view.name();
    Err(ApiError::conflict(format!(
        "{view_name} request branch must be based on {view_name} main"
    )))
}

fn commits_after(
    staging_repo: &FsPath,
    base: &str,
    new_head_oid: &str,
) -> Result<Vec<String>, ApiError> {
    let exclude_base = format!("^{base}");
    Ok(git_stdout_text(
        staging_repo,
        &[
            "rev-list",
            "--reverse",
            "--topo-order",
            new_head_oid,
            exclude_base.as_str(),
        ],
        "reading request branch commits",
    )?
    .lines()
    .filter(|line| !line.trim().is_empty())
    .map(ToString::to_string)
    .collect())
}

fn ensure_view_head_is_request_ancestor(
    staging_repo: &FsPath,
    request_head_oid: &str,
) -> Result<(), ApiError> {
    if git_is_ancestor(
        staging_repo,
        VIEW_REQUEST_BASE_REF,
        request_head_oid,
        "checking current request view main ancestry",
    )? {
        return Ok(());
    }
    Err(ApiError::conflict(VIEW_MAIN_MOVED_ERROR))
}

fn native_request_commit(
    staging_repo: &FsPath,
    commit_oid: &str,
    changed_paths: Vec<ScopePath>,
) -> Result<NativeRequestCommit, ApiError> {
    let tree_oid = git_stdout_text(
        staging_repo,
        &["show", "-s", "--format=%T", commit_oid],
        "reading request commit tree",
    )?
    .trim()
    .to_string();
    let parents = git_stdout_text(
        staging_repo,
        &["show", "-s", "--format=%P", commit_oid],
        "reading request commit parents",
    )?;
    Ok(NativeRequestCommit {
        oid: commit_oid.to_string(),
        parent_oids: parents
            .split_ascii_whitespace()
            .map(ToString::to_string)
            .collect(),
        tree_oid,
        changed_paths,
    })
}

fn git_commit_oid(staging_repo: &FsPath, revision: &str) -> Result<String, ApiError> {
    git_stdout_text(
        staging_repo,
        &["rev-parse", "--verify", &format!("{revision}^{{commit}}")],
        "reading current request view main",
    )
    .map(|oid| oid.trim().to_string())
}

fn validated_changed_paths_by_commit(
    staging_repo: &FsPath,
    commit_oids: Vec<String>,
) -> Result<Vec<(String, Vec<String>)>, ApiError> {
    commit_oids
        .into_iter()
        .map(|commit_oid| {
            validate_pushed_tree(staging_repo, &commit_oid)?;
            let paths = request_changed_paths(staging_repo, &commit_oid)?;
            Ok((commit_oid, paths))
        })
        .collect()
}

async fn changed_path_history(
    git: &RepositoryGit,
    state: &AppState,
    commits: &[(String, Vec<String>)],
) -> Result<PathHistory, ApiError> {
    let paths = commits
        .iter()
        .flat_map(|(_, paths)| paths)
        .filter_map(|path| ScopePath::parse(format!("/{path}")).ok())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    git.path_history(state, &paths).await
}

fn request_view_paths<'a>(
    request_view: RequestView<'a>,
    repo_config: &'a RepoConfig,
    visible_paths: &'a BTreeSet<String>,
    history: &'a PathHistory,
) -> RequestViewPaths<'a> {
    RequestViewPaths::new(
        repo_config,
        request_view.views,
        request_view.view,
        visible_paths,
        history,
    )
}

fn ensure_view_request_commit_paths(
    policy: &RequestViewPaths<'_>,
    paths: &[String],
) -> Result<Vec<ScopePath>, ApiError> {
    let mut changed_paths = BTreeSet::new();
    for path in paths {
        let scope_path = ScopePath::parse(format!("/{path}")).map_err(ApiError::bad_request)?;
        policy
            .ensure_editable(&scope_path)
            .map_err(|error| match error {
                RequestViewPathError::ProtectedPath => {
                    ApiError::protected_paths(vec![path.clone()])
                }
                RequestViewPathError::HiddenPath => {
                    ApiError::conflict(policy.rejection(&scope_path, error))
                }
            })?;
        changed_paths.insert(scope_path);
    }
    Ok(changed_paths.into_iter().collect())
}

fn request_changed_paths(staging_repo: &FsPath, commit_oid: &str) -> Result<Vec<String>, ApiError> {
    let view_base_oid = git_commit_oid(staging_repo, VIEW_REQUEST_BASE_REF)?;
    let parents = git_stdout_text(
        staging_repo,
        &["show", "-s", "--format=%P", commit_oid],
        "reading request commit parents",
    )?
    .split_ascii_whitespace()
    .map(ToString::to_string)
    .collect::<Vec<_>>();
    let diff_base = parents
        .iter()
        .find(|parent| parent.as_str() == view_base_oid)
        .or_else(|| parents.first())
        .ok_or_else(|| ApiError::conflict("request commit must have a parent"))?;
    let action = "reading request commit paths";
    let output = successful_git_output(
        run_git_output(
            Some(staging_repo),
            &[
                "diff",
                "-r",
                "--name-only",
                "-z",
                "--no-renames",
                diff_base,
                commit_oid,
            ],
            action,
        )?,
        action,
    )?;
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
    use std::{
        fs,
        path::{Path, PathBuf},
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn view_request_range_is_oldest_first_with_exact_git_facts() {
        let repo = initialized_repo("exact-range");
        fs::write(repo.join("agent.txt"), "base\n").unwrap();
        commit_all(&repo, "view base");
        let view_base = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", VIEW_REQUEST_BASE_REF, &view_base],
            "recording view request base",
        )
        .unwrap();

        fs::write(repo.join("agent.txt"), "first\n").unwrap();
        commit_all(&repo, "first request commit");
        let first = oid(&repo, "HEAD");
        fs::write(repo.join("second.txt"), "second\n").unwrap();
        commit_all(&repo, "second request commit");
        let second = oid(&repo, "HEAD");

        let commits = commits_after(&repo, VIEW_REQUEST_BASE_REF, &second).unwrap();
        assert_eq!(commits, [first.clone(), second.clone()]);

        let first_path = ScopePath::parse("/agent.txt").unwrap();
        let first_fact = native_request_commit(&repo, &first, vec![first_path.clone()]).unwrap();
        assert_eq!(first_fact.oid, first);
        assert_eq!(first_fact.parent_oids, [view_base]);
        assert_eq!(first_fact.changed_paths, [first_path]);
        assert_eq!(
            first_fact.tree_oid,
            oid(&repo, &format!("{}^{{tree}}", first_fact.oid))
        );

        let second_path = ScopePath::parse("/second.txt").unwrap();
        let second_fact = native_request_commit(&repo, &second, vec![second_path.clone()]).unwrap();
        assert_eq!(second_fact.oid, second);
        assert_eq!(second_fact.parent_oids, [first_fact.oid]);
        assert_eq!(second_fact.changed_paths, [second_path]);
        assert_eq!(
            second_fact.tree_oid,
            oid(&repo, &format!("{}^{{tree}}", second_fact.oid))
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn view_request_range_rejects_parent_outside_view_history_or_range() {
        let repo = initialized_repo("external-parent");
        fs::write(repo.join("agent.txt"), "base\n").unwrap();
        commit_all(&repo, "view base");
        let view_base = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", VIEW_REQUEST_BASE_REF, &view_base],
            "recording view request base",
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
            &["switch", "--create", "request", &view_base],
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

        let error = validated_view_parent_oids("Agent", &repo, &[request_commit, request_head])
            .unwrap_err();
        assert!(
            error
                .public_message()
                .contains("Agent request contains a parent outside Agent history")
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn merge_validation_rejects_request_without_current_view_head() {
        let repo = initialized_repo("stale-view-head");
        fs::write(repo.join("agent.txt"), "base\n").unwrap();
        commit_all(&repo, "view base");
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

        run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
        fs::write(repo.join("main.txt"), "advanced\n").unwrap();
        commit_all(&repo, "advance view main");
        let current_view_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", VIEW_REQUEST_BASE_REF, &current_view_head],
            "recording advanced view request base",
        )
        .unwrap();

        assert_eq!(
            ensure_view_head_is_request_ancestor(&repo, &request_head)
                .unwrap_err()
                .public_message(),
            VIEW_MAIN_MOVED_ERROR
        );

        let _ = fs::remove_dir_all(repo);
    }

    #[test]
    fn merge_path_validation_ignores_rules_inherited_from_current_view_main() {
        let repo = initialized_repo("inherited-view-rules");
        fs::write(repo.join("agent.txt"), "base\n").unwrap();
        commit_all(&repo, "view base");
        let original_base = oid(&repo, "HEAD");

        run_git(
            Some(&repo),
            &["switch", "--create", "request", &original_base],
            "creating request branch",
        )
        .unwrap();
        fs::write(repo.join("request.txt"), "request\n").unwrap();
        commit_all(&repo, "request change");

        run_git(Some(&repo), &["switch", "main"], "returning to view main").unwrap();
        fs::write(repo.join(".scope/RULES.md"), "maintainer rules\n").unwrap();
        commit_all(&repo, "update maintainer rules");
        let current_view_head = oid(&repo, "HEAD");
        run_git(
            Some(&repo),
            &["update-ref", VIEW_REQUEST_BASE_REF, &current_view_head],
            "recording advanced view request base",
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
            &["merge", "--no-ff", "main", "-m", "merge current view main"],
            "merging current view main",
        )
        .unwrap();
        let merge_oid = oid(&repo, "HEAD");

        let paths = request_changed_paths(&repo, &merge_oid).unwrap();

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
        let amended_merge_oid = oid(&repo, "HEAD");

        assert_eq!(
            request_changed_paths(&repo, &amended_merge_oid).unwrap(),
            [".scope/RULES.md", "request.txt"]
        );

        let _ = fs::remove_dir_all(repo);
    }

    fn initialized_repo(label: &str) -> PathBuf {
        let repo = temp_repo_path(label);
        run_git(
            None,
            &[
                "init",
                "--initial-branch=main",
                repo.to_string_lossy().as_ref(),
            ],
            "initializing view request safety test repository",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.name", "Test"],
            "configuring test name",
        )
        .unwrap();
        run_git(
            Some(&repo),
            &["config", "user.email", "test@scope.local"],
            "configuring test email",
        )
        .unwrap();
        fs::create_dir_all(repo.join(".scope")).unwrap();
        fs::write(repo.join(".scope/RULES.md"), []).unwrap();
        repo
    }

    fn commit_all(repo: &Path, message: &str) {
        run_git(Some(repo), &["add", "."], "staging safety test files").unwrap();
        run_git(
            Some(repo),
            &["commit", "-m", message],
            "committing safety test files",
        )
        .unwrap();
    }

    fn oid(repo: &Path, revision: &str) -> String {
        git_stdout_text(repo, &["rev-parse", revision], "reading safety test oid")
            .unwrap()
            .trim()
            .to_string()
    }

    fn temp_repo_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "scope-vcs-view-request-safety-{label}-{}-{nonce}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        path
    }
}
