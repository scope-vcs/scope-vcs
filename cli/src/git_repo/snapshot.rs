use super::*;

#[derive(Clone, Debug)]
pub struct GitSnapshot {
    branch: Option<String>,
    config: Vec<ConfigEntry>,
    remotes: Vec<RemoteUrls>,
}

#[derive(Clone, Debug)]
struct ConfigEntry {
    local: bool,
    key: String,
    value: String,
}

#[derive(Clone, Debug)]
struct RemoteUrls {
    name: String,
    fetch: Option<String>,
    push: Option<String>,
}

impl GitSnapshot {
    pub fn read(repo: &GitRepo) -> anyhow::Result<Self> {
        let branch = git_output_in_repo(repo, &["branch", "--show-current"])?;
        let config = git_output_in_repo(
            repo,
            &[
                "config",
                "--null",
                "--list",
                "--show-scope",
                "--show-origin",
            ],
        )?;
        if !config.status.success() {
            bail!(
                "read Git configuration failed: {}",
                String::from_utf8_lossy(&config.stderr).trim()
            );
        }
        let remotes = git_output_in_repo(repo, &["remote", "--verbose"])?;
        if !remotes.status.success() {
            bail!("list Git remotes failed");
        }
        let mut remotes = remote_urls(&String::from_utf8_lossy(&remotes.stdout));
        for remote in &mut remotes {
            if remote.fetch.is_none() {
                remote.fetch = Some(git_text_in_repo(
                    repo,
                    &["remote", "get-url", &remote.name],
                )?);
                if remote.push.is_none() {
                    remote.push = Some(git_text_in_repo(
                        repo,
                        &["remote", "get-url", "--push", &remote.name],
                    )?);
                }
            }
        }
        Ok(Self {
            branch: branch
                .status
                .success()
                .then(|| String::from_utf8_lossy(&branch.stdout).trim().to_string())
                .filter(|branch| !branch.is_empty()),
            config: config_entries(&String::from_utf8_lossy(&config.stdout)),
            remotes,
        })
    }

    pub fn branch(&self) -> Option<&str> {
        self.branch.as_deref()
    }

    pub fn branch_config_value(&self, branch: &str, key: &str) -> Option<&str> {
        self.value(&format!("branch.{branch}.{key}"), false)
    }

    pub fn scope_api_url(&self) -> Option<&str> {
        self.value(SCOPE_API_URL_CONFIG_KEY, true)
    }

    pub fn scope_git_origin(&self, fallback_url: &str) -> anyhow::Result<String> {
        match self.value(SCOPE_GIT_ORIGIN_CONFIG_KEY, true) {
            Some(origin) => Ok(origin.to_string()),
            None => transport_origin(fallback_url),
        }
    }

    pub fn remote_names(&self) -> impl Iterator<Item = &str> {
        self.remotes.iter().map(|remote| remote.name.as_str())
    }

    pub fn remote_fetch_url(&self, remote: &str) -> anyhow::Result<&str> {
        self.remote_url(remote, false)
    }

    pub fn remote_push_url(&self, remote: &str) -> anyhow::Result<&str> {
        self.remote_url(remote, true)
    }

    fn remote_url(&self, remote: &str, push: bool) -> anyhow::Result<&str> {
        let urls = self
            .remotes
            .iter()
            .find(|urls| urls.name == remote)
            .and_then(|urls| if push { &urls.push } else { &urls.fetch }.as_deref())
            .ok_or_else(|| {
                crate::error::CliError::usage(format!(
                    "Scope remote '{remote}' is not configured. Run: scope init"
                ))
            })?;
        if urls.is_empty() {
            let direction = if push { "push" } else { "fetch" };
            return Err(crate::error::CliError::usage(format!(
                "Scope remote '{remote}' has an empty {direction} URL"
            ))
            .into());
        }
        Ok(urls)
    }

    fn value(&self, key: &str, local_only: bool) -> Option<&str> {
        let key = normalized_config_key(key);
        self.config
            .iter()
            .rev()
            .find(|entry| entry.key == key && (entry.local || !local_only))
            .map(|entry| entry.value.trim())
            .filter(|value| !value.is_empty())
    }
}

fn normalized_config_key(key: &str) -> String {
    let (head, name) = key.rsplit_once('.').unwrap_or(("", key));
    match head.split_once('.') {
        Some((section, subsection)) => format!(
            "{}.{subsection}.{}",
            section.to_ascii_lowercase(),
            name.to_ascii_lowercase()
        ),
        None => format!(
            "{}.{}",
            head.to_ascii_lowercase(),
            name.to_ascii_lowercase()
        ),
    }
}

fn config_entries(listing: &str) -> Vec<ConfigEntry> {
    let mut fields = listing.split('\0');
    let mut local_file = None;
    let mut entries = Vec::new();
    while let (Some(scope), Some(origin), Some(entry)) =
        (fields.next(), fields.next(), fields.next())
    {
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let local = scope == "local" && *local_file.get_or_insert(origin) == origin;
        entries.push(ConfigEntry {
            local,
            key: key.to_string(),
            value: value.to_string(),
        });
    }
    entries
}

fn remote_urls(listing: &str) -> Vec<RemoteUrls> {
    let mut remotes: Vec<RemoteUrls> = Vec::new();
    for line in listing.lines() {
        let (name, url) = line.split_once('\t').unwrap_or((line, ""));
        if remotes.last().is_none_or(|remote| remote.name != name) {
            remotes.push(RemoteUrls {
                name: name.to_string(),
                fetch: None,
                push: None,
            });
        }
        let remote = remotes.last_mut().expect("remote was just pushed");
        if let Some(push) = url.strip_suffix(" (push)") {
            remote.push.get_or_insert_with(|| push.to_string());
        } else if let Some((fetch, _)) = url.rsplit_once(" (fetch)") {
            remote.fetch = Some(fetch.to_string());
        }
    }
    remotes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn snapshot_matches_git_lookup_scopes_and_url_rewrites() {
        let dir = TempDir::git_repo("git-snapshot", "Feature");
        let included = dir.path().join("included.gitconfig");
        std::fs::write(
            &included,
            "[scope]\n\tgitOrigin = https://included.example\n",
        )
        .unwrap();
        dir.run_git(["config", "include.path", included.to_str().unwrap()]);
        dir.run_git(["config", "scope.apiUrl", "https://api.example"]);
        dir.run_git(["config", "branch.Feature.scopeRequestId", "req_upper"]);
        dir.run_git(["config", "branch.feature.scopeRequestId", "req_lower"]);
        dir.run_git([
            "config",
            "url.https://rewritten.example/.insteadOf",
            "https://old.example/",
        ]);
        dir.run_git([
            "config",
            "url.https://push.example/.pushInsteadOf",
            "https://old.example/",
        ]);
        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://old.example/git/private/o/r",
        ]);
        dir.run_git(["remote", "add", "mirror", "https://mirror.example/o/r"]);
        dir.run_git([
            "config",
            "--add",
            "remote.mirror.pushurl",
            "https://first.example",
        ]);
        dir.run_git([
            "config",
            "--add",
            "remote.mirror.pushurl",
            "https://second.example",
        ]);
        dir.run_git([
            "config",
            "remote.alias.fetch",
            "+refs/heads/*:refs/remotes/alias/*",
        ]);
        dir.run_git([
            "config",
            "url.https://alias.example/git/private/o/r.insteadOf",
            "alias",
        ]);
        dir.run_git(["config", "remote.push-only.pushurl", "https://push.example"]);
        dir.run_git(["config", "remote.empty.url", ""]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        let snapshot = GitSnapshot::read(&repo).unwrap();

        assert_eq!(snapshot.branch(), Some("Feature"));
        assert_eq!(snapshot.scope_api_url(), Some("https://api.example"));
        assert_eq!(
            snapshot
                .scope_git_origin("https://fallback.example/x")
                .unwrap(),
            "https://fallback.example"
        );
        assert_eq!(
            snapshot.branch_config_value("Feature", "SCOPEREQUESTID"),
            Some("req_upper")
        );
        assert_eq!(
            snapshot.remote_names().collect::<Vec<_>>(),
            ["alias", "empty", "mirror", "push-only", "scope"]
        );
        for (remote, push) in [
            ("scope", false),
            ("scope", true),
            ("mirror", true),
            ("alias", false),
            ("alias", true),
            ("empty", false),
            ("empty", true),
            ("push-only", false),
            ("push-only", true),
        ] {
            let args = if push {
                vec!["remote", "get-url", "--push", remote]
            } else {
                vec!["remote", "get-url", remote]
            };
            let expected = git_text_in_repo(&repo, &args).unwrap();
            let actual = if push {
                snapshot.remote_push_url(remote)
            } else {
                snapshot.remote_fetch_url(remote)
            };
            assert_eq!(actual.unwrap(), expected, "{args:?}");
        }
        assert!(snapshot.remote_fetch_url("missing").is_err());
    }
}
