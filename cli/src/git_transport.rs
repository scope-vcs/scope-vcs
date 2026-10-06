use crate::git_repo::GitRepo;
use anyhow::{Context, bail};
use reqwest::Url;
use scope_domain::views::ViewId;

pub const DEFAULT_SCOPE_REMOTE: &str = "scope";
const LEGACY_FULL_VIEW_SEGMENT: &str = "permissioned";
const SCOPE_REMOTE_PATH_HINT: &str = "Scope remote must have path /git/<view>/owner/repo";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopeRemote {
    pub remote: String,
    pub view: ViewId,
    pub owner: String,
    pub repo: String,
    origin: Url,
}

impl ScopeRemote {
    pub fn parse(api_url: &str, name: &str, remote_url: &str) -> anyhow::Result<Self> {
        let api = Url::parse(api_url).context("parse Scope API URL")?;
        let remote = Url::parse(remote_url).context("parse Scope Git remote URL")?;

        if api.scheme() != remote.scheme()
            || api.host_str() != remote.host_str()
            || api.port_or_known_default() != remote.port_or_known_default()
        {
            bail!(
                "Scope remote points at {}, but this CLI is configured for {}",
                redacted_url(&remote),
                api.as_str().trim_end_matches('/')
            );
        }
        Self::parse_at_own_origin(name, remote)
    }

    pub fn from_url(remote_url: &str) -> anyhow::Result<Self> {
        Self::parse_at_own_origin(
            "",
            Url::parse(remote_url).context("parse Scope Git remote URL")?,
        )
    }

    fn parse_at_own_origin(name: &str, remote: Url) -> anyhow::Result<Self> {
        if remote.password().is_some() {
            bail!("Scope Git remote URL cannot include a password");
        }
        let segments = remote
            .path_segments()
            .map(|segments| segments.collect::<Vec<_>>())
            .unwrap_or_default();
        if segments.len() != 4 || segments[0] != "git" {
            bail!(SCOPE_REMOTE_PATH_HINT);
        }
        let view = ViewId::parse(segments[1])
            .map_err(|error| anyhow::anyhow!("{SCOPE_REMOTE_PATH_HINT}: {}", error.message))?;
        let owner = segments[2].trim();
        let repo = segments[3].trim();
        if owner.is_empty() || repo.is_empty() {
            bail!("Scope remote must include owner and repo");
        }
        let mut origin = remote.clone();
        let _ = origin.set_username("");
        origin.set_path("");
        origin.set_query(None);
        origin.set_fragment(None);

        Ok(Self {
            remote: name.to_string(),
            view,
            owner: owner.to_string(),
            repo: repo.to_string(),
            origin,
        })
    }

    pub fn url(&self) -> String {
        self.url_for_view(&self.view)
    }

    pub fn full_view_url(&self) -> String {
        self.url_for_view(&ViewId::private())
    }

    pub fn url_for_view(&self, view: &ViewId) -> String {
        let mut url = self.origin.clone();
        url.set_path(&format!("/git/{view}/{}/{}", self.owner, self.repo));
        url.to_string()
    }
}

pub fn legacy_full_view_url(remote_url: &str) -> Option<String> {
    let mut url = Url::parse(remote_url).ok()?;
    let segments = url.path_segments()?.map(str::to_owned).collect::<Vec<_>>();
    if segments.len() != 4 || segments[0] != "git" || segments[1] != LEGACY_FULL_VIEW_SEGMENT {
        return None;
    }
    url.set_path(&format!(
        "/git/{}/{}/{}",
        ViewId::PRIVATE,
        segments[2],
        segments[3]
    ));
    Some(url.to_string())
}

pub fn scope_path_origin(remote_url: &str) -> Option<String> {
    ScopeRemote::from_url(remote_url)
        .ok()
        .and_then(|remote| remote.origin.has_host().then(|| remote.origin.origin()))
        .map(|origin| origin.ascii_serialization())
}

pub fn select_scope_fetch_remote(
    repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
) -> anyhow::Result<String> {
    crate::context::select_remote(repo, api_url, explicit_remote, false)
}

pub fn select_scope_push_remote(
    repo: &GitRepo,
    api_url: &str,
    explicit_remote: Option<&str>,
) -> anyhow::Result<String> {
    crate::context::select_remote(repo, api_url, explicit_remote, true)
}

pub(crate) fn redacted_remote_url(remote_url: &str) -> String {
    Url::parse(remote_url)
        .map(|url| redacted_url(&url))
        .unwrap_or_else(|_| "an unparseable URL".into())
}

fn redacted_url(url: &Url) -> String {
    let mut redacted = url.clone();
    if !redacted.username().is_empty() {
        let _ = redacted.set_username("redacted");
    }
    if redacted.password().is_some() {
        let _ = redacted.set_password(Some("redacted"));
    }
    redacted.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn parses_the_view_from_the_remote_and_derives_every_view_address() {
        let remote = ScopeRemote::parse(
            "https://scope.example",
            "origin",
            "https://scope@scope.example/git/agent/adam/repo?ignored=true",
        )
        .unwrap();

        assert_eq!(remote.remote, "origin");
        assert_eq!(remote.view, ViewId::parse("agent").unwrap());
        assert_eq!(remote.owner, "adam");
        assert_eq!(remote.repo, "repo");
        assert_eq!(remote.url(), "https://scope.example/git/agent/adam/repo");
        assert_eq!(
            remote.full_view_url(),
            "https://scope.example/git/private/adam/repo"
        );
        assert_eq!(
            remote.url_for_view(&ViewId::public()),
            "https://scope.example/git/public/adam/repo"
        );
    }

    #[test]
    fn server_urls_are_parsed_at_their_own_origin() {
        let remote =
            ScopeRemote::from_url("https://git.scope.example/git/public/adam/repo").unwrap();
        assert!(remote.remote.is_empty());
        assert_eq!(
            remote.url_for_view(&ViewId::parse("agent").unwrap()),
            "https://git.scope.example/git/agent/adam/repo"
        );
    }

    #[test]
    fn only_legacy_permissioned_remotes_are_rewritten_to_the_full_view() {
        assert_eq!(
            legacy_full_view_url("https://scope@scope.example/git/permissioned/adam/repo")
                .as_deref(),
            Some("https://scope@scope.example/git/private/adam/repo")
        );
        for remote in [
            "https://scope.example/git/private/adam/repo",
            "https://scope.example/git/public/adam/repo",
            "https://scope.example/git/permissioned/adam",
            "https://github.com/adam/permissioned",
            "not a url",
        ] {
            assert_eq!(legacy_full_view_url(remote), None, "{remote}");
        }
    }

    #[test]
    fn mismatch_errors_redact_remote_credentials() {
        let error = ScopeRemote::parse(
            "https://scope.example",
            "origin",
            "https://scope:secret@evil.example/git/public/adam/repo",
        )
        .unwrap_err()
        .to_string();

        assert!(!error.contains("secret"), "{error}");
        assert!(error.contains("redacted:redacted"), "{error}");
    }

    #[test]
    fn rejects_passwords_and_non_scope_paths() {
        for remote in [
            "https://scope:secret@scope.example/git/private/adam/repo",
            "https://scope.example/adam/repo",
            "https://scope.example/git/Agent/adam/repo",
            "https://scope.example/git/1agent/adam/repo",
            "https://scope.example/git/public/adam",
            "https://scope.example/git/public/adam/repo/extra",
        ] {
            assert!(
                ScopeRemote::parse("https://scope.example", "origin", remote).is_err(),
                "accepted {remote}"
            );
        }
    }

    #[test]
    fn discovers_scope_remote_by_conventional_name_then_url() {
        let dir = TempDir::git_repo("scope-remote-discovery", "main");
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/private/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            select_scope_fetch_remote(&repo, "https://scope.example", None).unwrap(),
            "origin"
        );

        dir.run_git([
            "remote",
            "add",
            "scope",
            "https://scope.example/git/private/adam/repo",
        ]);
        assert_eq!(
            select_scope_fetch_remote(&repo, "https://scope.example", None).unwrap(),
            "scope"
        );
    }

    #[test]
    fn explicit_push_remote_uses_push_url() {
        let dir = TempDir::git_repo("scope-push-remote-discovery", "main");
        dir.run_git(["remote", "add", "origin", "https://github.com/adam/repo"]);
        dir.run_git([
            "remote",
            "set-url",
            "--push",
            "origin",
            "https://scope.example/git/private/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert!(select_scope_push_remote(&repo, "https://scope.example", None).is_err());
        assert!(select_scope_push_remote(&repo, "https://scope.example", Some("origin")).is_err());
    }

    #[test]
    fn configured_git_origin_can_differ_from_the_api_origin() {
        let dir = TempDir::git_repo("separate-scope-git-origin", "main");
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://git.scope.example/git/private/adam/repo",
        ]);
        dir.run_git(["config", "scope.gitOrigin", "https://git.scope.example"]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            select_scope_fetch_remote(&repo, "https://api.scope.example", None).unwrap(),
            "origin"
        );
        assert_eq!(
            select_scope_push_remote(&repo, "https://api.scope.example", None).unwrap(),
            "origin"
        );
    }

    #[test]
    fn push_discovery_skips_public_and_mismatched_fetch_remotes() {
        let dir = TempDir::git_repo("scope-push-safe-discovery", "main");
        dir.run_git([
            "remote",
            "add",
            "origin",
            "https://scope.example/git/public/adam/repo",
        ]);
        dir.run_git(["remote", "add", "github", "https://github.com/adam/repo"]);
        dir.run_git([
            "remote",
            "set-url",
            "--push",
            "github",
            "https://scope.example/git/private/adam/repo",
        ]);
        dir.run_git([
            "remote",
            "add",
            "upstream",
            "https://scope.example/git/private/adam/repo",
        ]);
        let repo = GitRepo {
            root: dir.path().to_path_buf(),
        };

        assert_eq!(
            select_scope_push_remote(&repo, "https://scope.example", None).unwrap(),
            "upstream"
        );
    }
}
