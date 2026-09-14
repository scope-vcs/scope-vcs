use crate::{GitSegmentRestoreTimings, GitStorageError};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, Weak},
    time::SystemTime,
};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VerifiedPackCacheUsage {
    pub retained_bytes: u64,
    pub leased_bytes: u64,
    pub pack_count: u64,
}

pub struct VerifiedGitPack {
    path: PathBuf,
    timings: GitSegmentRestoreTimings,
    _pin: VerifiedPackPin,
}

impl VerifiedGitPack {
    pub(crate) fn new(
        path: PathBuf,
        timings: GitSegmentRestoreTimings,
        pin: VerifiedPackPin,
    ) -> Self {
        Self {
            path,
            timings,
            _pin: pin,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn timings(&self) -> &GitSegmentRestoreTimings {
        &self.timings
    }

    pub(crate) fn into_publication(self) -> (GitSegmentRestoreTimings, VerifiedPackPin) {
        (self.timings, self._pin)
    }
}

impl std::fmt::Debug for VerifiedGitPack {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VerifiedGitPack")
            .field("path", &self.path)
            .field("timings", &self.timings)
            .finish_non_exhaustive()
    }
}

pub(crate) struct VerifiedPackCache {
    root: PathBuf,
    state: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<PathBuf, Entry>,
    flights: HashMap<PathBuf, Weak<VerifiedPackFlight>>,
}

struct Entry {
    leases: usize,
    last_used: SystemTime,
    verified_at: Option<SystemTime>,
}

impl CacheState {
    fn is_leased(&self, path: &Path) -> bool {
        self.entries.get(path).is_some_and(|entry| entry.leases > 0)
    }

    fn entry_mut(&mut self, path: &Path) -> &mut Entry {
        self.entries.entry(path.to_path_buf()).or_insert(Entry {
            leases: 0,
            last_used: SystemTime::now(),
            verified_at: None,
        })
    }

    fn clear_verified_at(&mut self, path: &Path) {
        if let Some(entry) = self.entries.get_mut(path) {
            entry.verified_at = None;
        }
    }
}

pub(crate) struct VerifiedPackFlight {
    outcome: Mutex<Option<FlightOutcome>>,
    publication_pin: Mutex<Option<VerifiedPackPin>>,
    completed: Notify,
}

type FlightOutcome = Result<GitSegmentRestoreTimings, Arc<GitStorageError>>;

pub(crate) struct VerifiedPackPin {
    cache: Arc<VerifiedPackCache>,
    path: PathBuf,
}

struct CacheEntry {
    pack_path: PathBuf,
    size_bytes: u64,
    last_used: SystemTime,
    has_pack: bool,
}

impl VerifiedPackCache {
    pub(crate) fn new(root: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            root,
            state: Mutex::new(CacheState::default()),
        })
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn lease_existing(
        self: &Arc<Self>,
        path: &Path,
        expected_bytes: u64,
        expected_sha256: &str,
        timings: GitSegmentRestoreTimings,
    ) -> Result<Option<VerifiedGitPack>, GitStorageError> {
        let mut state = self.lock_state();
        if !validate_existing(&mut state, path, expected_bytes, expected_sha256)? {
            return Ok(None);
        }
        let pin = self.pin_locked(&mut state, path.to_path_buf());
        Ok(Some(VerifiedGitPack::new(path.to_path_buf(), timings, pin)))
    }

    pub(crate) fn install(
        self: &Arc<Self>,
        source: &Path,
        path: &Path,
        expected_bytes: u64,
        expected_sha256: &str,
    ) -> Result<VerifiedPackPin, GitStorageError> {
        let source_metadata = fs::metadata(source).map_err(GitStorageError::Local)?;
        if !source_metadata.is_file() {
            return Err(GitStorageError::Local(std::io::Error::other(
                "verified Git pack source is not a file",
            )));
        }
        if source_metadata.len() != expected_bytes {
            return Err(GitStorageError::SizeMismatch {
                expected: expected_bytes,
                actual: source_metadata.len(),
            });
        }

        let mut state = self.lock_state();
        if validate_existing(&mut state, path, expected_bytes, expected_sha256)? {
            remove_file_if_exists(source)?;
        } else {
            fs::rename(source, path).map_err(GitStorageError::Local)?;
            // Ingest or remote hydration already authenticated this source.
            let modified = source_metadata.modified().map_err(GitStorageError::Local)?;
            state.entry_mut(path).verified_at = Some(modified);
        }
        Ok(self.pin_locked(&mut state, path.to_path_buf()))
    }

    pub(crate) fn begin_flight(self: &Arc<Self>, path: &Path) -> (Arc<VerifiedPackFlight>, bool) {
        let mut state = self.lock_state();
        if let Some(flight) = state.flights.get(path).and_then(Weak::upgrade) {
            return (flight, false);
        }
        let flight = Arc::new(VerifiedPackFlight {
            outcome: Mutex::new(None),
            publication_pin: Mutex::new(None),
            completed: Notify::new(),
        });
        state
            .flights
            .insert(path.to_path_buf(), Arc::downgrade(&flight));
        (flight, true)
    }

    pub(crate) fn finish_flight(&self, path: &Path, completed: &Arc<VerifiedPackFlight>) {
        let mut state = self.lock_state();
        if state
            .flights
            .get(path)
            .and_then(Weak::upgrade)
            .is_some_and(|current| Arc::ptr_eq(&current, completed))
        {
            state.flights.remove(path);
        }
    }

    pub(crate) fn usage(&self) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        let state = self.lock_state();
        usage_for_entries(&cache_entries(&self.root, &state)?, &state)
    }

    pub(crate) fn evict_to(
        &self,
        target_bytes: u64,
    ) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        let mut state = self.lock_state();
        let mut entries = cache_entries(&self.root, &state)?;
        entries.sort_by_key(|entry| entry.last_used);
        // Only unleased entries are evicted, so leased_bytes stays exact.
        let mut usage = usage_for_entries(&entries, &state)?;
        for entry in entries {
            if usage.retained_bytes <= target_bytes {
                break;
            }
            if state.is_leased(&entry.pack_path) {
                continue;
            }
            remove_cache_artifacts(&entry.pack_path)?;
            state.entries.remove(&entry.pack_path);
            usage.retained_bytes = usage.retained_bytes.saturating_sub(entry.size_bytes);
            if entry.has_pack {
                usage.pack_count = usage.pack_count.saturating_sub(1);
            }
        }
        Ok(usage)
    }

    fn pin_locked(self: &Arc<Self>, state: &mut CacheState, path: PathBuf) -> VerifiedPackPin {
        let entry = state.entry_mut(&path);
        entry.leases += 1;
        entry.last_used = SystemTime::now();
        VerifiedPackPin {
            cache: Arc::clone(self),
            path,
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, CacheState> {
        lock_recovering(&self.state)
    }
}

// Cache state is a set of counters and timestamps that stay consistent across
// a panic in another holder, so recover the guard rather than failing.
fn lock_recovering<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl VerifiedPackFlight {
    /// Records the outcome and wakes waiters. The first outcome wins; later
    /// calls are ignored so a panic guard cannot overwrite a real result.
    pub(crate) fn complete(
        &self,
        result: Result<(GitSegmentRestoreTimings, VerifiedPackPin), GitStorageError>,
    ) {
        let mut stored = lock_recovering(&self.outcome);
        if stored.is_some() {
            return;
        }
        *stored = Some(match result {
            Ok((timings, pin)) => {
                *lock_recovering(&self.publication_pin) = Some(pin);
                Ok(timings)
            }
            Err(error) => Err(Arc::new(error)),
        });
        drop(stored);
        self.completed.notify_waiters();
    }

    pub(crate) async fn wait(&self) -> FlightOutcome {
        loop {
            let notified = self.completed.notified();
            if let Some(outcome) = lock_recovering(&self.outcome).as_ref() {
                return outcome.clone();
            }
            notified.await;
        }
    }
}

impl Drop for VerifiedPackPin {
    fn drop(&mut self) {
        let mut state = self.cache.lock_state();
        // A leased entry is never removed, so it is always present here.
        if let Some(entry) = state.entries.get_mut(&self.path) {
            entry.leases = entry.leases.saturating_sub(1);
            entry.last_used = SystemTime::now();
        }
    }
}

fn cache_entries(root: &Path, state: &CacheState) -> Result<Vec<CacheEntry>, GitStorageError> {
    let mut grouped = BTreeMap::<PathBuf, CacheEntry>::new();
    let repositories = match fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(GitStorageError::Local(error)),
    };
    for repository in repositories {
        let repository = repository.map_err(GitStorageError::Local)?;
        if !repository
            .file_type()
            .map_err(GitStorageError::Local)?
            .is_dir()
            || repository.file_name() == ".tmp"
        {
            continue;
        }
        for artifact in fs::read_dir(repository.path()).map_err(GitStorageError::Local)? {
            let artifact = artifact.map_err(GitStorageError::Local)?;
            let metadata = artifact.metadata().map_err(GitStorageError::Local)?;
            if !metadata.is_file() {
                continue;
            }
            let artifact_path = artifact.path();
            let extension = artifact_path.extension().and_then(|value| value.to_str());
            let pack_path = match extension {
                Some("pack") => artifact_path.clone(),
                Some("idx") => artifact_path.with_extension("pack"),
                _ => continue,
            };
            let last_used = state
                .entries
                .get(&pack_path)
                .map(|entry| entry.last_used)
                .or_else(|| metadata.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            let entry = grouped.entry(pack_path.clone()).or_insert(CacheEntry {
                pack_path,
                size_bytes: 0,
                last_used,
                has_pack: false,
            });
            entry.size_bytes = entry
                .size_bytes
                .checked_add(metadata.len())
                .ok_or_else(cache_size_overflow)?;
            entry.last_used = entry.last_used.max(last_used);
            entry.has_pack |= extension == Some("pack");
        }
    }
    Ok(grouped.into_values().collect())
}

fn usage_for_entries(
    entries: &[CacheEntry],
    state: &CacheState,
) -> Result<VerifiedPackCacheUsage, GitStorageError> {
    let mut usage = VerifiedPackCacheUsage::default();
    for entry in entries {
        usage.retained_bytes = usage
            .retained_bytes
            .checked_add(entry.size_bytes)
            .ok_or_else(cache_size_overflow)?;
        if entry.has_pack {
            usage.pack_count = usage
                .pack_count
                .checked_add(1)
                .ok_or_else(cache_size_overflow)?;
        }
        if state.is_leased(&entry.pack_path) {
            usage.leased_bytes = usage
                .leased_bytes
                .checked_add(entry.size_bytes)
                .ok_or_else(cache_size_overflow)?;
        }
    }
    Ok(usage)
}

// Retained files are rechecked once per process, and again if their modification
// time changes. Published immutable files can otherwise be leased without I/O
// proportional to pack size. Callers run this filesystem work off the executor.
fn validate_existing(
    state: &mut CacheState,
    path: &Path,
    expected_bytes: u64,
    expected_sha256: &str,
) -> Result<bool, GitStorageError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => metadata,
        Ok(_) => {
            return Err(GitStorageError::Local(std::io::Error::other(
                "verified Git pack cache path is not a file",
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            state.clear_verified_at(path);
            return Ok(false);
        }
        Err(error) => return Err(GitStorageError::Local(error)),
    };
    let modified = metadata.modified().map_err(GitStorageError::Local)?;
    let mismatch = if metadata.len() != expected_bytes {
        Some(GitStorageError::SizeMismatch {
            expected: expected_bytes,
            actual: metadata.len(),
        })
    } else if state.entries.get(path).and_then(|entry| entry.verified_at) != Some(modified) {
        let mut file = fs::File::open(path).map_err(GitStorageError::Local)?;
        let mut digest = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let bytes = file.read(&mut buffer).map_err(GitStorageError::Local)?;
            if bytes == 0 {
                break;
            }
            digest.update(&buffer[..bytes]);
        }
        let actual = hex::encode(digest.finalize());
        (actual != expected_sha256).then(|| GitStorageError::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual,
        })
    } else {
        None
    };
    if let Some(error) = mismatch {
        state.clear_verified_at(path);
        if state.is_leased(path) {
            return Err(error);
        }
        remove_cache_artifacts(path)?;
        state.entries.remove(path);
        return Ok(false);
    }
    state.entry_mut(path).verified_at = Some(modified);
    Ok(true)
}

fn remove_cache_artifacts(pack_path: &Path) -> Result<(), GitStorageError> {
    remove_file_if_exists(pack_path)?;
    remove_file_if_exists(&pack_path.with_extension("idx"))
}

fn remove_file_if_exists(path: &Path) -> Result<(), GitStorageError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(GitStorageError::Local(error)),
    }
}

fn cache_size_overflow() -> GitStorageError {
    GitStorageError::Task("verified Git pack cache size overflow".into())
}
