use crate::git::import::require_git_success;
use crate::{
    config::DEFAULT_GIT_BRANCH,
    error::ApiError,
    git::import::{run_git, run_git_output},
    persistence::ensure_private_dir,
};
use scope_domain::repository::RepositoryIncarnation;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

mod coordinator;
pub(crate) use coordinator::{GitDerivedCacheCoordinator, GitDerivedCacheNamespace};

const STALE_MATERIALIZATION_MAX_IDLE: Duration = Duration::from_secs(30 * 60);
const REPOSITORY_GIT_CACHE_TOUCH_INTERVAL: Duration = Duration::from_secs(60);
const APPLIED_FRONTIER_FILE: &str = "scope-cache-applied-frontier";
const LAST_USED_FILE: &str = "scope-cache-last-used";

pub(crate) struct RepositoryGitCache {
    root: PathBuf,
    max_bytes: usize,
}

#[derive(Debug)]
pub(crate) struct GitRepoHandle {
    path: PathBuf,
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
struct AppliedRepositoryFrontier {
    version: u8,
    incarnation: RepositoryIncarnation,
    push_sequence: u64,
}

fn cache_users() -> &'static Mutex<BTreeMap<PathBuf, usize>> {
    static USERS: OnceLock<Mutex<BTreeMap<PathBuf, usize>>> = OnceLock::new();
    USERS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

impl RepositoryGitCache {
    pub(crate) fn new(root: PathBuf, max_bytes: usize) -> Result<Arc<Self>, ApiError> {
        if max_bytes == 0 {
            return Err(ApiError::internal_message(
                "repository Git cache byte budget must be greater than zero",
            ));
        }
        ensure_private_dir(&root)?;
        let registry = Arc::new(Self { root, max_bytes });
        registry.prune()?;
        Ok(registry)
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn path_for(&self, incarnation: &RepositoryIncarnation) -> PathBuf {
        self.root.join(format!(
            "repo-{}.git",
            repository_git_cache_key(incarnation)
        ))
    }

    pub(crate) fn lease(
        self: &Arc<Self>,
        incarnation: &RepositoryIncarnation,
    ) -> Result<GitRepoHandle, ApiError> {
        self.lease_path(self.path_for(incarnation))
    }

    pub(crate) fn lease_derived(
        self: &Arc<Self>,
        path: PathBuf,
    ) -> Result<GitRepoHandle, ApiError> {
        let is_direct_child = path.parent() == Some(self.root.as_path());
        let is_git_repository = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.ends_with(".git"));
        if !is_direct_child || !is_git_repository {
            return Err(ApiError::internal_message(
                "derived Git cache path is outside the managed cache root",
            ));
        }
        self.lease_path(path)
    }

    fn lease_path(self: &Arc<Self>, path: PathBuf) -> Result<GitRepoHandle, ApiError> {
        {
            let mut users = cache_users().lock().map_err(|_| {
                ApiError::internal_message("repository Git cache registry is poisoned")
            })?;
            touch_if_materialized(&path)?;
            *users.entry(path.clone()).or_default() += 1;
        }
        Ok(GitRepoHandle { path })
    }

    pub(crate) fn note_applied(
        &self,
        incarnation: &RepositoryIncarnation,
        path: &Path,
        push_sequence: u64,
    ) -> Result<(), ApiError> {
        let frontier = AppliedRepositoryFrontier {
            version: 1,
            incarnation: incarnation.clone(),
            push_sequence,
        };
        let bytes = serde_json::to_vec(&frontier).map_err(ApiError::internal)?;
        fs::write(path.join(APPLIED_FRONTIER_FILE), bytes).map_err(ApiError::internal)?;
        touch_if_materialized(path)
    }

    pub(crate) fn applied_sequence(
        &self,
        incarnation: &RepositoryIncarnation,
        path: &Path,
    ) -> Option<u64> {
        fs::read(path.join(APPLIED_FRONTIER_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AppliedRepositoryFrontier>(&bytes).ok())
            .filter(|frontier| frontier.version == 1 && &frontier.incarnation == incarnation)
            .map(|frontier| frontier.push_sequence)
    }

    pub(crate) fn remove(&self, incarnation: &RepositoryIncarnation) -> Result<bool, ApiError> {
        let path = self.path_for(incarnation);
        evict_unleased(&path, remove_dir_if_exists)
    }

    pub(crate) fn prune(&self) -> Result<(), ApiError> {
        let mut caches = repository_cache_directories(&self.root)?;
        let now = SystemTime::now();
        prune_stale_materializations(&self.root, now, STALE_MATERIALIZATION_MAX_IDLE)?;
        caches.sort_by_key(|entry| entry.last_used);

        let mut retained_bytes = caches
            .iter()
            .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
            .ok_or_else(|| ApiError::internal_message("repository Git cache size overflow"))?;
        let max_bytes = self.max_bytes as u64;
        let mut evicted_bytes = 0_u64;
        let mut evicted_repositories = 0_u64;
        for entry in caches {
            if retained_bytes > max_bytes && evict_unleased(&entry.path, remove_dir_if_exists)? {
                retained_bytes = retained_bytes.saturating_sub(entry.size_bytes);
                evicted_bytes += entry.size_bytes;
                evicted_repositories += 1;
            }
        }
        if evicted_repositories > 0 {
            tracing::info!(
                reason = "size_pressure",
                evicted_repositories,
                evicted_bytes,
                retained_bytes,
                max_bytes,
                "repository Git caches evicted"
            );
        }
        Ok(())
    }
}

// Only the rename needs to exclude new leases. Recursive deletion happens at
// a detached path so a slow disk never holds the registry for unrelated repos.
fn evict_unleased(
    path: &Path,
    remove: impl FnOnce(&Path) -> Result<(), ApiError>,
) -> Result<bool, ApiError> {
    static EVICTION: AtomicU64 = AtomicU64::new(1);
    let retired = {
        let users = cache_users()
            .lock()
            .map_err(|_| ApiError::internal_message("repository Git cache registry is poisoned"))?;
        if users.get(path).copied().unwrap_or_default() > 0 {
            return Ok(false);
        }
        let retired = path.with_extension(format!(
            "evicting.{}.{}.tmp",
            std::process::id(),
            EVICTION.fetch_add(1, Ordering::Relaxed),
        ));
        match fs::rename(path, &retired) {
            Ok(()) => retired,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(error) => return Err(ApiError::internal(error)),
        }
    };
    remove(&retired)?;
    Ok(true)
}

pub(crate) fn sanitize_repository_git_cache_repo(
    repo: &Path,
    expected_head: &str,
) -> Result<(), ApiError> {
    let output = run_git_output(
        Some(repo),
        &["for-each-ref", "--format=%(refname)%00%(objectname)"],
        "reading refs before repository Git cache synchronization",
    )?;
    let output = require_git_success(
        output,
        "reading refs before repository Git cache synchronization",
    )?;
    let refs = String::from_utf8(output.stdout).map_err(ApiError::internal)?;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let mut found_main = false;
    for line in refs.lines() {
        let (refname, oid) = line.split_once('\0').ok_or_else(|| {
            ApiError::internal_message("invalid repository Git cache ref listing")
        })?;
        if refname == main_ref {
            if oid != expected_head {
                return Err(ApiError::internal_message(
                    "repository Git cache main ref does not match committed head",
                ));
            }
            found_main = true;
        } else {
            run_git(
                Some(repo),
                &["update-ref", "-d", refname],
                "removing non-main ref before repository Git cache synchronization",
            )?;
        }
    }
    if !found_main {
        return Err(ApiError::internal_message(
            "repository Git cache is missing the committed main ref",
        ));
    }
    Ok(())
}

fn prune_stale_materializations(
    root: &Path,
    now: SystemTime,
    max_idle: Duration,
) -> Result<(), ApiError> {
    for entry in fs::read_dir(root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".tmp") || !path.is_dir() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if now
            .duration_since(modified)
            .is_ok_and(|idle| idle >= max_idle)
        {
            remove_dir_if_exists(&path)?;
        }
    }
    Ok(())
}

impl Deref for GitRepoHandle {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for GitRepoHandle {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

impl Drop for GitRepoHandle {
    fn drop(&mut self) {
        if let Ok(mut users) = cache_users().lock() {
            match users.get_mut(&self.path) {
                Some(count) if *count > 1 => *count -= 1,
                Some(_) => {
                    users.remove(&self.path);
                }
                None => {}
            }
        }
        if let Err(error) = touch_if_materialized(&self.path) {
            tracing::warn!(
                path = %self.path.display(),
                error = %error.operator_diagnostic(),
                "failed to prune local repository Git caches"
            );
        }
    }
}

fn repository_git_cache_key(incarnation: &RepositoryIncarnation) -> String {
    let mut hasher = Sha256::new();
    for value in [
        incarnation.repository_id().as_bytes(),
        incarnation.incarnation_id().as_bytes(),
    ] {
        hasher.update((value.len() as u64).to_be_bytes());
        hasher.update(value);
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..16])
}

fn touch_if_materialized(path: &Path) -> Result<(), ApiError> {
    if path.is_dir() {
        let marker = path.join(LAST_USED_FILE);
        let touched_recently = fs::metadata(&marker)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|elapsed| elapsed < REPOSITORY_GIT_CACHE_TOUCH_INTERVAL);
        if !touched_recently {
            fs::write(marker, []).map_err(ApiError::internal)?;
        }
    }
    Ok(())
}

struct RepositoryCacheEntry {
    path: PathBuf,
    last_used: SystemTime,
    size_bytes: u64,
}

fn repository_cache_directories(root: &Path) -> Result<Vec<RepositoryCacheEntry>, ApiError> {
    let mut caches = Vec::new();
    for entry in fs::read_dir(root).map_err(ApiError::internal)? {
        let entry = entry.map_err(ApiError::internal)?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.ends_with(".git") || !path.is_dir() {
            continue;
        }
        let last_used = fs::metadata(path.join(LAST_USED_FILE))
            .or_else(|_| fs::metadata(&path))
            .and_then(|metadata| metadata.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        caches.push(RepositoryCacheEntry {
            size_bytes: directory_size(&path)?,
            path,
            last_used,
        });
    }
    Ok(caches)
}

fn directory_size(root: &Path) -> Result<u64, ApiError> {
    let mut total = 0_u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(ApiError::internal(error)),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ApiError::internal(error)),
            };
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(ApiError::internal(error)),
            };
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(|| {
                    ApiError::internal_message("repository Git cache size overflow")
                })?;
            }
        }
    }
    Ok(total)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), ApiError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ApiError::internal(error)),
    }
}

#[cfg(test)]
mod tests;
