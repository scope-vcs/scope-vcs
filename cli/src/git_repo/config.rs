use super::*;

pub fn install_scope_fetch_auth(
    repo_root: &Path,
    remote_url: &str,
    api_url: &str,
) -> anyhow::Result<()> {
    let helper_key = credential_config_key(remote_url, "helper")?;
    let use_http_path_key = credential_config_key(remote_url, "useHttpPath")?;
    let git_origin = transport_origin(remote_url)?;
    let unset = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local", "--unset-all", &helper_key])
        .status()
        .context("clear existing Scope Git credential helpers")?;
    if !unset.success() && unset.code() != Some(5) {
        bail!("clear existing Scope Git credential helpers failed");
    }
    run_git_config(repo_root, &["--add", &helper_key, ""])?;
    run_git_config(
        repo_root,
        &["--add", &helper_key, SCOPE_GIT_CREDENTIAL_HELPER],
    )?;
    run_git_config(repo_root, &["--replace-all", &use_http_path_key, "true"])?;
    run_git_config(
        repo_root,
        &["--replace-all", SCOPE_API_URL_CONFIG_KEY, api_url],
    )?;
    run_git_config(
        repo_root,
        &["--replace-all", SCOPE_GIT_ORIGIN_CONFIG_KEY, &git_origin],
    )?;
    Ok(())
}

pub fn scope_git_origin(repo: &GitRepo, fallback_url: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(
        repo,
        &["config", "--local", "--get", SCOPE_GIT_ORIGIN_CONFIG_KEY],
    )?;
    if output.status.success() {
        let origin = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !origin.is_empty() {
            return Ok(origin);
        }
    } else if output.status.code() != Some(1) {
        bail!("read configured Scope Git origin failed");
    }
    transport_origin(fallback_url)
}

pub fn scope_api_url_from_git_config(repo_root: &Path) -> anyhow::Result<Option<String>> {
    let output = Command::new("git")
        .current_dir(repo_root)
        .env("LC_ALL", "C")
        .args(["config", "--local", "--get", SCOPE_API_URL_CONFIG_KEY])
        .output()
        .context("read configured Scope API URL")?;
    if output.status.success() {
        let api_url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!api_url.is_empty()).then_some(api_url));
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(1)
        || stderr.trim() == "fatal: --local can only be used inside a git repository"
    {
        return Ok(None);
    }
    bail!(
        "read configured Scope API URL failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn transport_origin(value: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(value).context("parse Scope transport URL")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_path("");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

fn run_git_config(repo_root: &Path, args: &[&str]) -> anyhow::Result<()> {
    let status = Command::new("git")
        .current_dir(repo_root)
        .args(["config", "--local"])
        .args(args)
        .status()
        .context("configure Scope Git credential helper")?;
    if !status.success() {
        bail!("configure Scope Git credential helper failed");
    }
    Ok(())
}

fn credential_config_key(remote_url: &str, name: &str) -> anyhow::Result<String> {
    if remote_url.chars().any(char::is_control) {
        bail!("Scope Git remote URL cannot contain control characters");
    }
    Ok(format!("credential.{remote_url}.{name}"))
}

pub fn configure_scope_push_address(
    repo_root: &Path,
    remote: &str,
    fetch_url: &str,
) -> anyhow::Result<()> {
    let Ok(scope_remote) = crate::git_transport::ScopeRemote::from_url(fetch_url) else {
        return Ok(());
    };
    let push_url_key = format!("remote.{remote}.pushurl");
    if scope_remote.view.is_private() {
        let unset = Command::new("git")
            .current_dir(repo_root)
            .args(["config", "--local", "--unset-all", &push_url_key])
            .status()
            .context("clear Scope push address")?;
        if !unset.success() && unset.code() != Some(5) {
            bail!("clear Scope push address failed");
        }
        return Ok(());
    }
    run_git_config(
        repo_root,
        &[
            "--replace-all",
            &push_url_key,
            &scope_remote.full_view_url(),
        ],
    )
}

pub fn git_remote_push_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    remote_url(repo, remote, true)
}

pub fn git_remote_fetch_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    remote_url(repo, remote, false)
}

fn remote_url(repo: &GitRepo, remote: &str, push: bool) -> anyhow::Result<String> {
    let url = configured_remote_url(repo, remote, push)?;
    if crate::git_transport::legacy_full_view_url(&url).is_none() {
        return Ok(url);
    }
    migrate_legacy_full_view_remote(repo, remote)?;
    let url = configured_remote_url(repo, remote, push)?;
    if crate::git_transport::legacy_full_view_url(&url).is_some() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' uses a /git/permissioned/ address outside this repository's Git config; point it at /git/private/ with git remote set-url"
        ))
        .into());
    }
    Ok(url)
}

fn configured_remote_url(repo: &GitRepo, remote: &str, push: bool) -> anyhow::Result<String> {
    let args = if push {
        vec!["remote", "get-url", "--push", remote]
    } else {
        vec!["remote", "get-url", remote]
    };
    let output = git_output_in_repo(repo, &args)?;
    if !output.status.success() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' is not configured. Run: scope init"
        ))
        .into());
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        let direction = if push { "push" } else { "fetch" };
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' has an empty {direction} URL"
        ))
        .into());
    }
    Ok(url)
}

fn migrate_legacy_full_view_remote(repo: &GitRepo, remote: &str) -> anyhow::Result<()> {
    for key in ["url", "pushurl"] {
        let config_key = format!("remote.{remote}.{key}");
        for legacy in local_config_values(repo, &config_key)? {
            let Some(current) = crate::git_transport::legacy_full_view_url(&legacy) else {
                continue;
            };
            run_git_in_repo(
                repo,
                &[
                    "config",
                    "--local",
                    "--fixed-value",
                    "--replace-all",
                    &config_key,
                    &current,
                    &legacy,
                ],
            )?;
            move_credential_section(repo, &legacy, &current)?;
            eprintln!(
                "Scope Git addresses now name the view: updated remote {} {key} to {}",
                crate::display::terminal_text(remote),
                crate::git_transport::redacted_remote_url(&current)
            );
        }
    }
    Ok(())
}

fn move_credential_section(repo: &GitRepo, legacy: &str, current: &str) -> anyhow::Result<()> {
    let (legacy, current) = (transport_url(legacy)?, transport_url(current)?);
    if local_config_values(repo, &credential_config_key(&legacy, "helper")?)?.is_empty() {
        return Ok(());
    }
    run_git_in_repo(
        repo,
        &[
            "config",
            "--local",
            "--rename-section",
            &format!("credential.{legacy}"),
            &format!("credential.{current}"),
        ],
    )
}

fn transport_url(value: &str) -> anyhow::Result<String> {
    let mut url = Url::parse(value).context("parse Scope transport URL")?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn local_config_values(repo: &GitRepo, key: &str) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(repo, &["config", "--local", "--get-all", key])?;
    if output.status.code() == Some(1) {
        return Ok(Vec::new());
    }
    if !output.status.success() {
        bail!("read Git config {key} failed");
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

pub fn git_remote_names(repo: &GitRepo) -> anyhow::Result<Vec<String>> {
    let output = git_output_in_repo(repo, &["remote"])?;
    if !output.status.success() {
        bail!("list Git remotes failed");
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|remote| !remote.is_empty())
        .map(str::to_string)
        .collect())
}

pub fn branch_config_value(
    repo: &GitRepo,
    branch: &str,
    key: &str,
) -> anyhow::Result<Option<String>> {
    let config_key = format!("branch.{branch}.{key}");
    let output = git_output_in_repo(repo, &["config", "--get", &config_key])?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        return Ok((!value.is_empty()).then_some(value));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    bail!(
        "read branch configuration failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

pub fn set_branch_config_value(
    repo: &GitRepo,
    branch: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    let config_key = format!("branch.{branch}.{key}");
    run_git_in_repo(repo, &["config", "--local", &config_key, value])
}
