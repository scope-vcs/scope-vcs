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

pub fn git_remote_push_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["remote", "get-url", "--push", remote])?;
    if !output.status.success() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' is not configured. Run: scope init"
        ))
        .into());
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' has an empty push URL"
        ))
        .into());
    }
    Ok(url)
}

pub fn git_remote_fetch_url(repo: &GitRepo, remote: &str) -> anyhow::Result<String> {
    let output = git_output_in_repo(repo, &["remote", "get-url", remote])?;
    if !output.status.success() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' is not configured. Run: scope init"
        ))
        .into());
    }

    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return Err(crate::error::CliError::usage(format!(
            "Scope remote '{remote}' has an empty fetch URL"
        ))
        .into());
    }
    Ok(url)
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
