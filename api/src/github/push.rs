//! Pushing a request's tested commit to its GitHub branch, and deleting the
//! branch. The installation token reaches git as an HTTP header through git's
//! environment, never in argv or the URL, so it shows in no process list and
//! in none of git's error output.

use super::GitHubApp;
use crate::{
    error::ApiError,
    git::command::{git_process_output, truncated_git_stderr},
};
use base64::Engine;
use scope_git_process::ProcessLimits;
use std::{path::Path, process::Command, time::Duration};

/// Long enough to send a large repository's history the first time.
const PUSH_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// One GitHub repository, as the app pushes it.
pub(crate) struct GitHubPushRemote {
    url: String,
    authorization: String,
}

impl GitHubApp {
    pub(crate) fn push_remote(
        &self,
        full_name: &str,
        installation_token: &str,
    ) -> GitHubPushRemote {
        let credentials = base64::engine::general_purpose::STANDARD
            .encode(format!("x-access-token:{installation_token}"));
        GitHubPushRemote {
            url: format!("{}/{full_name}.git", self.git_url),
            authorization: format!("Authorization: Basic {credentials}"),
        }
    }
}

impl GitHubPushRemote {
    /// Points `git_ref` at `oid` from `repo`, replacing whatever it held. The
    /// error is what GitHub answered.
    pub(crate) fn push(&self, repo: &Path, oid: &str, git_ref: &str) -> Result<(), String> {
        self.run(repo, &format!("{oid}:{git_ref}"))
    }

    /// Deletes `git_ref`. A branch that is already gone counts as deleted.
    pub(crate) fn delete(&self, git_ref: &str) -> Result<(), String> {
        let scratch = tempfile::tempdir()
            .map_err(|error| format!("could not prepare the branch deletion: {error}"))?;
        self.git(scratch.path(), &["init", "--quiet", "--bare", "."])?;
        match self.run(scratch.path(), &format!(":{git_ref}")) {
            Err(error) if error.contains("remote ref does not exist") => Ok(()),
            result => result,
        }
    }

    fn run(&self, repo: &Path, refspec: &str) -> Result<(), String> {
        self.git(
            repo,
            &["push", "--force", "--porcelain", &self.url, refspec],
        )
        .map_err(|error| format!("GitHub refused the push: {error}"))
    }

    fn git(&self, repo: &Path, args: &[&str]) -> Result<(), String> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(repo)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_CONFIG_COUNT", "2")
            .env("GIT_CONFIG_KEY_0", "http.extraHeader")
            .env("GIT_CONFIG_VALUE_0", &self.authorization)
            // No credential helper may answer for, or store, this token.
            .env("GIT_CONFIG_KEY_1", "credential.helper")
            .env("GIT_CONFIG_VALUE_1", "");
        let output = git_process_output(&mut command, None, ProcessLimits::new(PUSH_TIMEOUT))
            .map_err(ApiError::into_operator_diagnostic)?;
        if output.status.success() {
            return Ok(());
        }
        // `--porcelain` reports a rejected ref on stdout; transport errors go to stderr.
        let stdout = String::from_utf8_lossy(&output.stdout);
        let rejected = stdout
            .lines()
            .filter(|line| line.starts_with('!'))
            .collect::<Vec<_>>()
            .join("; ");
        let stderr = truncated_git_stderr(&output.stderr);
        Err([rejected.trim(), stderr.trim()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("; "))
    }
}
