//! Resolve repository and endpoint identity for every CLI delivery path.
use crate::{
    error::CliError,
    git_repo::{
        GitRepo, branch_config_value, current_branch, git_remote_fetch_url, git_remote_names,
        git_remote_push_url, scope_api_url_from_git_config, scope_git_origin,
    },
    git_transport::{DEFAULT_SCOPE_REMOTE, GitAccess, ScopeRemote},
};
use anyhow::Context;
use std::{env, path::PathBuf, process::Command};

pub fn explicit_repository() -> Option<&'static str> {
    crate::execution::options().repository.as_deref()
}

pub fn validate_api_url(value: &str) -> anyhow::Result<()> {
    let url = reqwest::Url::parse(value).map_err(|_| {
        CliError::usage("Scope API URL must be an absolute http:// or https:// URL")
    })?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CliError::usage(
            "Scope API URL must use HTTP(S) and cannot contain credentials, a query, or a fragment",
        )
        .into());
    }
    Ok(())
}

pub fn api_url(default: &str) -> anyhow::Result<String> {
    let explicit = crate::execution::options()
        .api_url
        .clone()
        .or_else(|| env::var("SCOPE_API_URL").ok())
        .or_else(|| env::var("SCOPE_API_PUBLIC_URL").ok());
    let endpoint = match explicit {
        Some(endpoint) => endpoint,
        None => scope_api_url_from_git_config(
            &env::current_dir().context("inspect current directory")?,
        )?
        .unwrap_or_else(|| default.to_string()),
    };
    Ok(endpoint.trim_end_matches('/').to_string())
}

pub fn discover_optional() -> anyhow::Result<Option<GitRepo>> {
    discover_at(&env::current_dir().context("inspect current directory")?)
}

fn discover_at(cwd: &std::path::Path) -> anyhow::Result<Option<GitRepo>> {
    let output = Command::new("git")
        .current_dir(cwd)
        .env("LC_ALL", "C")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("inspect Git repository")?;
    if !output.status.success() {
        let error = String::from_utf8_lossy(&output.stderr);
        if error.starts_with("fatal: not a git repository (or any of the parent directories): .git")
            || error.starts_with("fatal: not a git repository (or any parent up to mount point ")
        {
            return Ok(None);
        }
        anyhow::bail!("inspect Git repository failed: {}", error.trim());
    }
    let root = String::from_utf8(output.stdout).context("Git repository path is not UTF-8")?;
    // Git terminates its output with a newline; spaces are part of the path.
    let root = root.trim_end_matches(['\r', '\n']);
    if root.is_empty() {
        return Err(CliError::usage("Git repository root is empty").into());
    }
    Ok(Some(GitRepo {
        root: PathBuf::from(root),
    }))
}

pub fn resolve_repository(
    repo: Option<&GitRepo>,
    explicit_remote: Option<&str>,
) -> anyhow::Result<ScopeRemote> {
    resolve(
        repo,
        &crate::api::api_url()?,
        explicit_remote,
        explicit_repository(),
        false,
    )
}

pub(crate) fn select_remote(
    repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
    push: bool,
) -> anyhow::Result<String> {
    let target = resolve(
        Some(repo),
        api_url,
        explicit_remote,
        explicit_repository(),
        push,
    )?;
    if target.remote.is_empty() {
        return Err(CliError::usage(
            "this Git operation requires a configured Scope remote; pass --remote <name>",
        )
        .into());
    }
    Ok(target.remote)
}

fn resolve(
    repo: Option<&GitRepo>,
    api_url: &str,
    explicit_remote: Option<&str>,
    explicit_repo: Option<&str>,
    push: bool,
) -> anyhow::Result<ScopeRemote> {
    validate_api_url(api_url)?;
    let selected = explicit_repo
        .map(crate::clone::parse_repo_spec)
        .transpose()?;
    let remote_arg = explicit_remote.map(str::trim).filter(|s| !s.is_empty());
    let Some(repo) = repo else {
        if remote_arg.is_some() {
            return Err(CliError::usage(
                "--remote requires a Git checkout; use --repo owner/repo outside a checkout",
            )
            .into());
        }
        let selected = selected.ok_or_else(|| {
            CliError::usage("run inside a Scope Git repository or pass --repo owner/repo")
        })?;
        return target_for_repository(api_url, &selected.owner, &selected.repo);
    };
    if let Some(stored_api) = scope_api_url_from_git_config(&repo.root)?
        && stored_api.trim_end_matches('/') != api_url.trim_end_matches('/')
    {
        return Err(CliError::usage("the selected API endpoint conflicts with this checkout's scope.apiUrl; use the matching checkout or correct its environment configuration").into());
    }
    let origin = scope_git_origin(repo, api_url)?;
    let load = |name: &str, validate_push: bool| -> anyhow::Result<ScopeRemote> {
        let fetch = ScopeRemote::parse(&origin, name, &git_remote_fetch_url(repo, name)?)?;
        let target = if validate_push {
            let push = ScopeRemote::parse(&origin, name, &git_remote_push_url(repo, name)?)?;
            if (fetch.owner.as_str(), fetch.repo.as_str())
                != (push.owner.as_str(), push.repo.as_str())
            {
                return Err(CliError::usage(format!("remote {name} fetches and pushes different Scope repositories; correct its URLs before pushing")).into());
            }
            if push.access != GitAccess::Permissioned {
                return Err(CliError::usage(format!(
                    "remote {name} must use a permissioned push URL"
                ))
                .into());
            }
            push
        } else {
            fetch
        };
        if let Some(expected) = &selected
            && (target.owner.as_str(), target.repo.as_str())
                != (expected.owner.as_str(), expected.repo.as_str())
        {
            return Err(CliError::usage(format!(
                "remote {name} targets {}/{}, which conflicts with --repo {}/{}",
                target.owner, target.repo, expected.owner, expected.repo
            ))
            .into());
        }
        Ok(target)
    };
    if let Some(remote) = remote_arg {
        return load(remote, push);
    }
    // Explicit repository selection bypasses implicit branch context.
    if selected.is_none()
        && let Ok(branch) = current_branch(repo)
    {
        if let Some(remote) = branch_config_value(repo, &branch, "scopeRequestRemote")? {
            return load(&remote, push);
        }
        if let Some(remote) = branch_config_value(repo, &branch, "remote")?
            && load(&remote, false).is_ok()
        {
            return load(&remote, push);
        }
    }
    let mut candidates = git_remote_names(repo)?
        .into_iter()
        .filter_map(|name| load(&name, false).ok())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        if let Some(selected) = selected
            && !push
        {
            return target_for_repository(api_url, &selected.owner, &selected.repo);
        }
        return Err(CliError::usage(
            "no Scope Git remote found; pass --remote <name> or run scope init",
        )
        .into());
    }
    let first = (&candidates[0].owner, &candidates[0].repo);
    if candidates
        .iter()
        .any(|target| (&target.owner, &target.repo) != first)
    {
        return Err(CliError::usage("multiple Scope repositories are configured; pass --remote <name> or --repo owner/repo to choose one").into());
    }
    if push {
        candidates = candidates
            .into_iter()
            .filter_map(|target| load(&target.remote, true).ok())
            .collect();
        if candidates.is_empty() {
            return Err(CliError::usage(
                "no Scope Git remote has a permissioned push URL for the selected repository",
            )
            .into());
        }
    }
    candidates.sort_by_key(|target| match target.remote.as_str() {
        DEFAULT_SCOPE_REMOTE => 0,
        "origin" => 1,
        _ => 2,
    });
    Ok(candidates.remove(0))
}

fn target_for_repository(api_url: &str, owner: &str, repo: &str) -> anyhow::Result<ScopeRemote> {
    let mut origin = reqwest::Url::parse(api_url)?;
    origin.set_path("");
    let url = origin.join(&scope_api_contract::routes::git_repo(
        "permissioned",
        owner,
        repo,
    ))?;
    ScopeRemote::parse(origin.as_str().trim_end_matches('/'), "", url.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TestDir;

    fn checkout(label: &str) -> (TestDir, GitRepo) {
        let dir = TestDir::git_repo(label, "main");
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };
        (dir, repo)
    }

    #[test]
    fn endpoint_discovery_distinguishes_no_checkout_from_broken_config() {
        let outside = crate::test_support::TestDir::new("endpoint-outside");
        assert!(
            scope_api_url_from_git_config(outside.path())
                .unwrap()
                .is_none()
        );
        let dir = crate::test_support::TestDir::git_repo("endpoint-config", "main");
        assert!(scope_api_url_from_git_config(dir.path()).unwrap().is_none());
        dir.run_git(["config", "scope.apiUrl", "https://local.example"]);
        assert_eq!(
            scope_api_url_from_git_config(dir.path())
                .unwrap()
                .as_deref(),
            Some("https://local.example")
        );
        std::fs::write(dir.path().join(".git/config"), "[broken").unwrap();
        let error = scope_api_url_from_git_config(dir.path()).unwrap_err();
        assert!(format!("{error:#}").contains("bad config"), "{error:#}");
        assert!(discover_at(dir.path()).is_err());
        let bare = crate::test_support::TestDir::new("endpoint-bare");
        bare.run_git(["init", "--bare"]);
        bare.run_git(["config", "scope.apiUrl", "https://bare.example"]);
        assert_eq!(
            scope_api_url_from_git_config(bare.path())
                .unwrap()
                .as_deref(),
            Some("https://bare.example")
        );
    }

    #[test]
    fn ambiguous_repositories_fail_before_filtering_push_permissions() {
        let (dir, repo) = checkout("context-ambiguous");
        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://scope.example/git/public/owner/one",
        ]);
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/permissioned/owner/two",
        ]);
        for push in [false, true] {
            let error =
                resolve(Some(&repo), "https://scope.example", None, None, push).unwrap_err();
            assert!(error.to_string().contains("multiple Scope repositories"));
            assert_eq!(crate::error::exit_code(&error), 2);
        }
    }

    #[test]
    fn explicit_target_and_branch_tracking_share_one_selector() {
        let (dir, repo) = checkout("context-priority");
        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://scope.example/git/permissioned/owner/one",
        ]);
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/permissioned/owner/two",
        ]);
        dir.run_git(["config", "branch.main.remote", "origin"]);
        assert_eq!(
            resolve(Some(&repo), "https://scope.example", None, None, false)
                .unwrap()
                .repo,
            "two"
        );
        assert_eq!(
            resolve(
                Some(&repo),
                "https://scope.example",
                None,
                Some("owner/one"),
                false
            )
            .unwrap()
            .repo,
            "one"
        );
        assert!(
            resolve(
                Some(&repo),
                "https://scope.example",
                Some("origin"),
                Some("owner/one"),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn remote_only_repository_needs_no_checkout() {
        let target = resolve(
            None,
            "https://api.scope.example",
            None,
            Some("owner/repo"),
            false,
        )
        .unwrap();
        assert_eq!(target.owner, "owner");
        assert!(target.remote.is_empty());
        assert!(
            resolve(
                None,
                "https://api.scope.example",
                Some("scope"),
                Some("owner/repo"),
                false
            )
            .is_err()
        );
    }

    #[test]
    fn conflicting_environment_refuses_checkout_operations() {
        let (dir, repo) = checkout("context-environment");
        dir.run_git(["config", "scope.apiUrl", "https://api.staging.example"]);
        dir.run_git(["config", "scope.gitOrigin", "https://git.staging.example"]);
        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://git.staging.example/git/permissioned/owner/repo",
        ]);
        assert!(
            resolve(
                Some(&repo),
                "https://api.production.example",
                None,
                None,
                false
            )
            .is_err()
        );
        assert!(
            resolve(
                Some(&repo),
                "https://api.staging.example",
                None,
                None,
                false
            )
            .is_ok()
        );
    }

    #[test]
    fn endpoint_validation_refuses_credentials_and_non_http_urls() {
        for url in [
            "https://user@scope.example",
            "https://scope.example?token=value",
            "file:///tmp/socket",
            "not-a-url",
        ] {
            assert!(validate_api_url(url).is_err());
        }
        assert!(validate_api_url("http://127.0.0.1:8080").is_ok());
    }
}
