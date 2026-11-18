use crate::{
    git_repo::{
        GitRepo, GitSnapshot, git_output_in_repo, git_text_in_repo, install_scope_fetch_auth,
        run_git_in_repo,
    },
    git_transport::{ScopeRemote, scope_path_origin},
};
use anyhow::bail;
use std::collections::BTreeMap;

pub(super) fn move_other_remotes_to_git_origin(
    repo: &GitRepo,
    snapshot: &GitSnapshot,
    configured: &ScopeRemote,
    target: &ScopeRemote,
    api_url: &str,
) -> anyhow::Result<()> {
    if configured.shares_origin_with(target) {
        return Ok(());
    }
    let rewrites = UrlRewrites::read(repo)?;
    let new_origin = scope_path_origin(&target.url()).unwrap_or_else(|| target.url());
    for name in snapshot
        .remote_names()
        .filter(|name| *name != target.remote)
    {
        let urls = RemoteUrls::read(repo, name)?;
        let Some(writes) = urls.exact_move(&rewrites, |url| match ScopeRemote::from_url(url) {
            Ok(other) if other.shares_origin_with(configured) => other
                .at_origin_of(&target.url())
                .ok()
                .map(|moved| moved.url()),
            _ => None,
        }) else {
            eprintln!(
                "Remote {name} still uses the retired Scope Git origin. Point its URLs at {new_origin} with `git remote set-url`."
            );
            continue;
        };
        for write in writes {
            let new = match &write {
                Write::Replace { key, old, new } => {
                    run_git_in_repo(
                        repo,
                        &[
                            "config",
                            "--local",
                            "--fixed-value",
                            "--replace-all",
                            key,
                            new,
                            old,
                        ],
                    )?;
                    new
                }
                Write::AddPushUrl { key, new } => {
                    run_git_in_repo(repo, &["config", "--local", "--add", key, new])?;
                    new
                }
            };
            install_scope_fetch_auth(&repo.root, new, api_url)?;
        }
    }
    Ok(())
}

enum Write {
    Replace {
        key: String,
        old: String,
        new: String,
    },
    AddPushUrl {
        key: String,
        new: String,
    },
}

struct UrlRewrites {
    fetch: Vec<String>,
    push: Vec<String>,
}

impl UrlRewrites {
    fn read(repo: &GitRepo) -> anyhow::Result<Self> {
        let output = git_output_in_repo(
            &repo.root,
            &[
                "config",
                "--null",
                "--get-regexp",
                r"^url\..*\.(insteadof|pushinsteadof)$",
            ],
        )?;
        let mut rewrites = Self {
            fetch: Vec::new(),
            push: Vec::new(),
        };
        for (key, prefix) in String::from_utf8_lossy(&output.stdout)
            .split('\0')
            .filter_map(|entry| entry.split_once('\n'))
        {
            if key.ends_with(".pushinsteadof") {
                rewrites.push.push(prefix.to_owned());
            } else {
                rewrites.fetch.push(prefix.to_owned());
            }
        }
        Ok(rewrites)
    }

    fn rewrites_for_push(&self, url: &str) -> bool {
        self.push.iter().any(|prefix| url.starts_with(prefix))
    }

    fn rewrites(&self, url: &str) -> bool {
        self.rewrites_for_push(url) || self.fetch.iter().any(|prefix| url.starts_with(prefix))
    }
}

struct RemoteUrls {
    url_key: String,
    pushurl_key: String,
    urls: Vec<String>,
    pushurls: Vec<String>,
    local_urls: Vec<String>,
    local_pushurls: Vec<String>,
    fetched: Vec<String>,
    pushed: Vec<String>,
}

impl RemoteUrls {
    fn read(repo: &GitRepo, name: &str) -> anyhow::Result<Self> {
        let url_key = format!("remote.{name}.url");
        let pushurl_key = format!("remote.{name}.pushurl");
        let urls = config_values(repo, &[&url_key])?;
        let pushurls = config_values(repo, &[&pushurl_key])?;
        let fetched = effective_urls(repo, name, false)?;
        let pushed = effective_urls(repo, name, true)?;
        let aligned =
            |raw: &[String], effective: &[String]| raw.is_empty() || raw.len() == effective.len();
        if !aligned(&urls, &fetched) || !aligned(&pushurls, &pushed) {
            bail!("resolve the URLs of Git remote {name} failed");
        }
        Ok(Self {
            local_urls: config_values(repo, &["--local", &url_key])?,
            local_pushurls: config_values(repo, &["--local", &pushurl_key])?,
            url_key,
            pushurl_key,
            urls,
            pushurls,
            fetched,
            pushed,
        })
    }

    fn exact_move(
        &self,
        rewrites: &UrlRewrites,
        moved: impl Fn(&str) -> Option<String>,
    ) -> Option<Vec<Write>> {
        let mut replacements = BTreeMap::new();
        let mut replace = |key: &str, local: &[String], raw: &str, effective: &str| {
            let Some(new) = moved(effective) else {
                return Some(());
            };
            if !local.iter().any(|value| value == raw) || rewrites.rewrites(&new) {
                return None;
            }
            replacements.insert((key.to_owned(), raw.to_owned()), new);
            Some(())
        };
        for (raw, effective) in self.urls.iter().zip(&self.fetched) {
            replace(&self.url_key, &self.local_urls, raw, effective)?;
        }
        for (raw, effective) in self.pushurls.iter().zip(&self.pushed) {
            replace(&self.pushurl_key, &self.local_pushurls, raw, effective)?;
        }
        let mut writes = Vec::new();
        let push_follows_urls = !self.pushurls.is_empty()
            || !self.urls.iter().any(|url| rewrites.rewrites_for_push(url));
        if !push_follows_urls {
            let pinned: Vec<_> = self.pushed.iter().map(|url| moved(url)).collect();
            if !replacements.is_empty() || pinned.iter().any(Option::is_some) {
                for new in pinned {
                    let new = new.filter(|new| !rewrites.rewrites(new))?;
                    writes.push(Write::AddPushUrl {
                        key: self.pushurl_key.clone(),
                        new,
                    });
                }
            }
        }
        writes.extend(
            replacements
                .into_iter()
                .map(|((key, old), new)| Write::Replace { key, old, new }),
        );
        Some(writes)
    }
}

fn config_values(repo: &GitRepo, selector: &[&str]) -> anyhow::Result<Vec<String>> {
    let mut args = vec!["config", "--get-all"];
    args.extend(selector);
    let output = git_output_in_repo(&repo.root, &args)?;
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

fn effective_urls(repo: &GitRepo, remote: &str, push: bool) -> anyhow::Result<Vec<String>> {
    let mut args = vec!["remote", "get-url", "--all"];
    if push {
        args.push("--push");
    }
    args.push(remote);
    Ok(git_text_in_repo(&repo.root, &args)?
        .lines()
        .map(str::to_owned)
        .collect())
}
