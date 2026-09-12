use super::*;

pub fn push_head_with_bearer(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
) -> anyhow::Result<()> {
    let plan = git_push_auth_plan(
        destination,
        commit_oid,
        branch,
        bearer_token,
        push_intent_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope git push",
        "git push to Scope failed",
    )
}

pub fn push_head_to_ref_with_bearer(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_push_ref_auth_plan(
        destination,
        commit_oid,
        refname,
        bearer_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope request branch push",
        "git push to Scope request ref failed",
    )
}

pub fn clone_with_bearer(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
) -> anyhow::Result<()> {
    let plan = git_clone_auth_plan(
        remote_url,
        bearer_token,
        destination,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        None,
        "run authenticated Scope git clone",
        "git clone from Scope failed",
    )
}

pub fn fetch_scope_remote_with_bearer(
    repo: &GitRepo,
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
) -> anyhow::Result<()> {
    let plan = git_fetch_auth_plan(
        destination,
        remote,
        branch,
        bearer_token,
        inherited_git_config_count(),
    );
    run_git_plan_output(
        plan,
        Some(&repo.root),
        "refresh Scope Git remote before push review",
        "refresh Scope Git remote before push review failed",
    )
}

pub fn git_push_ref_auth_plan(
    destination: &str,
    commit_oid: &str,
    refname: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:{refname}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_clone_auth_plan(
    remote_url: &str,
    bearer_token: &str,
    destination: Option<&Path>,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let mut args = vec!["clone".to_string(), remote_url.to_string()];
    if let Some(destination) = destination {
        args.push(destination.to_string_lossy().to_string());
    }
    git_auth_plan(
        args,
        remote_url,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

pub fn git_push_auth_plan(
    destination: &str,
    commit_oid: &str,
    branch: &str,
    bearer_token: &str,
    push_intent_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "push.recurseSubmodules=no".to_string(),
            "push".to_string(),
            destination.to_string(),
            format!("{commit_oid}:refs/heads/{branch}"),
        ],
        destination,
        &[
            format!("Authorization: Bearer {bearer_token}"),
            format!("X-Scope-Push-Intent: {push_intent_token}"),
        ],
        inherited_config_count,
    )
}

pub fn git_fetch_auth_plan(
    destination: &str,
    remote: &str,
    branch: &str,
    bearer_token: &str,
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    git_auth_plan(
        vec![
            "-c".to_string(),
            "protocol.version=2".to_string(),
            "fetch".to_string(),
            "--no-tags".to_string(),
            destination.to_string(),
            format!("+refs/heads/{branch}:refs/remotes/{remote}/{branch}"),
        ],
        destination,
        &[format!("Authorization: Bearer {bearer_token}")],
        inherited_config_count,
    )
}

fn git_auth_plan(
    args: Vec<String>,
    destination: &str,
    headers: &[String],
    inherited_config_count: Option<usize>,
) -> GitCommandPlan {
    let first_index = inherited_config_count.unwrap_or(0);
    let mut env = vec![(
        "GIT_CONFIG_COUNT".to_string(),
        (first_index + headers.len()).to_string(),
    )];
    for (offset, header) in headers.iter().enumerate() {
        let index = first_index + offset;
        env.push((
            format!("GIT_CONFIG_KEY_{index}"),
            format!("http.{destination}.extraHeader"),
        ));
        env.push((format!("GIT_CONFIG_VALUE_{index}"), header.clone()));
    }
    GitCommandPlan { args, env }
}

fn inherited_git_config_count() -> Option<usize> {
    env::var("GIT_CONFIG_COUNT")
        .ok()
        .and_then(|value| value.parse().ok())
}

fn git_command(plan: GitCommandPlan, cwd: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    command.args(plan.args);
    command.envs(plan.env);
    command
}

fn run_git_plan_output(
    plan: GitCommandPlan,
    cwd: Option<&Path>,
    context: &str,
    failure: &str,
) -> anyhow::Result<()> {
    let output = git_command(plan, cwd)
        .output()
        .with_context(|| context.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            bail!("{failure}: {stderr}");
        }
        bail!("{failure}");
    }
    Ok(())
}
