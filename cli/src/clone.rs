use crate::api::ApiSession;
use crate::{
    api::{api_url, get_repo, http_client},
    auth::read_stored_session_token,
    git_repo::{clone_with_bearer, configure_scope_push_address, install_scope_fetch_auth},
    git_transport::ScopeRemote,
    repo_config::write_worktree_scope_repo_config_with_base,
    repository_views::reader_repo_config,
};
use crate::{error::CliError, execution::emit};
use scope_domain::{repo_config::RepoConfig, views::ViewId};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, PartialEq, Eq)]
pub struct RepoSpec {
    pub owner: String,
    pub repo: String,
}

pub fn clone_repo(
    repository: &str,
    view: Option<&ViewId>,
    destination: Option<&Path>,
) -> anyhow::Result<()> {
    let target = parse_repo_spec(repository)?;
    let api_url = api_url()?;
    let session_token = read_stored_session_token(&api_url)?
        .ok_or_else(|| CliError::authentication("not signed in; run scope login"))?;
    let client = http_client()?;
    let api = ApiSession::new(&client, &api_url, &session_token);
    let repo = get_repo(api, &target.owner, &target.repo)?;
    let repo_config = reader_repo_config(api, &target.owner, &target.repo, &repo)?;
    let remote_url = clone_url(&repo.git_remote_url, &repo_config, view)?;
    let checkout_dir = destination
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_clone_dir(&target.repo));

    eprintln!(
        "Clone {}/{} into {}",
        target.owner,
        target.repo,
        checkout_dir.display()
    );
    clone_and_configure(
        &api_url,
        &remote_url,
        &session_token,
        &checkout_dir,
        &repo_config,
    )?;
    emit(
        "clone",
        &json!({"repository": format!("{}/{}", target.owner, target.repo), "directory": checkout_dir, "remote_url": remote_url, "configured": true}),
        vec![format!(
            "Cloned {}/{} into {}",
            target.owner,
            target.repo,
            checkout_dir.display()
        )],
    )
}

fn clone_url(
    reader_url: &str,
    config: &RepoConfig,
    view: Option<&ViewId>,
) -> anyhow::Result<String> {
    let Some(view) = view else {
        return Ok(reader_url.to_string());
    };
    if config.views().get(view).is_none() {
        return Err(CliError::usage(format!(
            "Unknown repository view {view}; choose one of {}",
            config
                .views()
                .iter()
                .map(|definition| definition.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
        .into());
    }
    Ok(ScopeRemote::from_url(reader_url)?.url_for_view(view))
}

fn clone_and_configure(
    api_url: &str,
    remote_url: &str,
    session_token: &str,
    checkout_dir: &Path,
    config: &RepoConfig,
) -> anyhow::Result<()> {
    clone_with_bearer(remote_url, session_token, Some(checkout_dir))?;
    install_scope_fetch_auth(checkout_dir, remote_url, api_url)
        .and_then(|_| configure_scope_push_address(checkout_dir, "origin", remote_url))
        .and_then(|_| write_worktree_scope_repo_config_with_base(checkout_dir, config))
        .map_err(|error| CliError::partial(
            format!("Clone completed at {}, but local Scope setup failed: {error:#}", checkout_dir.display()),
            json!({"operation": "clone", "cloned": true, "configured": false, "directory": checkout_dir, "remote_url": remote_url, "recovery": "Keep this checkout. Run scope doctor from it to inspect local Scope configuration and fix the reported Git or filesystem error. Run scope pull to restore fetch authentication and any missing local visibility state, then scope visibility show to inspect it before publishing. Do not repeat clone into this directory."})
        ).into())
}

pub fn parse_repo_spec(repository: &str) -> anyhow::Result<RepoSpec> {
    let repository = repository.trim();
    if repository.contains("://") {
        return Err(CliError::usage("expected repository as owner/repo").into());
    }

    let mut parts = repository.split('/');
    let owner = parts.next().unwrap_or_default().trim();
    let repo = parts.next().unwrap_or_default().trim();
    if owner.is_empty() || repo.is_empty() || parts.next().is_some() {
        return Err(CliError::usage("expected repository as owner/repo").into());
    }

    Ok(RepoSpec {
        owner: owner.to_string(),
        repo: repo.to_string(),
    })
}

pub fn default_clone_dir(repo: &str) -> PathBuf {
    repo.strip_suffix(".git")
        .filter(|name| !name.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(repo))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repo_config::{
            default_scope_repo_config, load_worktree_scope_repo_config, repo_config_path,
        },
        test_support::TempDir,
    };
    use std::{fs, process::Command};

    #[test]
    fn clone_defaults_to_the_reader_view_and_switches_to_a_requested_view() {
        let config = default_scope_repo_config();
        let reader = "https://git.scope.example/git/public/adam/repo";
        assert_eq!(clone_url(reader, &config, None).unwrap(), reader);
        assert_eq!(
            clone_url(reader, &config, Some(&ViewId::private())).unwrap(),
            "https://git.scope.example/git/private/adam/repo"
        );
        let error = clone_url(reader, &config, Some(&ViewId::parse("agent").unwrap()))
            .unwrap_err()
            .to_string();
        assert!(error.contains("Unknown repository view agent"), "{error}");
    }

    #[test]
    fn clone_installs_fetch_auth_and_repo_config() {
        let dir = TempDir::git_repo("clone-orchestration", "main");
        fs::write(dir.path().join("README.md"), "initial\n").unwrap();
        dir.run_git(["add", "README.md"]);
        dir.run_git([
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit",
            "--quiet",
            "-m",
            "initial",
        ]);
        let checkout = dir.path().join("checkout");
        let remote_url = format!("file://{}", dir.path().display());
        let config = default_scope_repo_config();

        clone_and_configure(
            "https://api.scope.example",
            &remote_url,
            "secret",
            &checkout,
            &config,
        )
        .unwrap();

        assert_eq!(load_worktree_scope_repo_config(&checkout).unwrap(), config);
        assert!(repo_config_path(&checkout).unwrap().is_file());
        assert!(!checkout.join(".scope").exists());
        let helper = Command::new("git")
            .current_dir(&checkout)
            .args([
                "config",
                "--local",
                "--get-urlmatch",
                "credential.helper",
                &remote_url,
            ])
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&helper.stdout).trim(),
            "!scope git-credential"
        );
    }
    #[test]
    fn clone_keeps_checkout_and_reports_receipt_when_local_setup_fails() {
        let dir = TempDir::git_repo("clone-partial", "main");
        dir.run_git([
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit",
            "--allow-empty",
            "--quiet",
            "-m",
            "initial",
        ]);
        let checkout = dir.path().join("checkout");
        let error = clone_and_configure(
            "https://scope.example",
            dir.path().to_str().unwrap(),
            "secret",
            &checkout,
            &default_scope_repo_config(),
        )
        .unwrap_err();
        assert!(checkout.join(".git").is_dir());
        assert_eq!(crate::error::exit_code(&error), 5);
        assert!(error.to_string().contains("Clone completed"));
        let receipt = serde_json::to_value(crate::error::json_response(&error)).unwrap();
        assert_eq!(receipt["recovery"]["cloned"], true);
        assert_eq!(receipt["recovery"]["configured"], false);
    }
}
