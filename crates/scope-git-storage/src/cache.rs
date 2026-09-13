use crate::{GitSegmentRestoreTimings, GitStorageError};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap},
    fs,
    io::Read,
    ops::Deref,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
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

impl Deref for VerifiedGitPack {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl AsRef<Path> for VerifiedGitPack {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

pub(crate) struct VerifiedPackCache {
    root: PathBuf,
    state: Mutex<CacheState>,
}

#[derive(Default)]
struct CacheState {
    leases: HashMap<PathBuf, usize>,
    last_used: HashMap<PathBuf, SystemTime>,
    flights: HashMap<PathBuf, Weak<VerifiedPackFlight>>,
    verified_at: HashMap<PathBuf, SystemTime>,
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
        let mut state = self.lock_state()?;
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

        let mut state = self.lock_state()?;
        if validate_existing(&mut state, path, expected_bytes, expected_sha256)? {
            remove_file_if_exists(source)?;
        } else {
            fs::rename(source, path).map_err(GitStorageError::Local)?;
            // Ingest or remote hydration already authenticated this source.
            state.verified_at.insert(
                path.to_path_buf(),
                source_metadata.modified().map_err(GitStorageError::Local)?,
            );
        }
        Ok(self.pin_locked(&mut state, path.to_path_buf()))
    }

    pub(crate) fn begin_flight(self: &Arc<Self>, path: &Path) -> (Arc<VerifiedPackFlight>, bool) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
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
        let Ok(mut state) = self.state.lock() else {
            return;
        };
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
        let state = self.lock_state()?;
        usage_for_entries(&cache_entries(&self.root, &state)?, &state)
    }

    pub(crate) fn evict_to(
        &self,
        target_bytes: u64,
    ) -> Result<VerifiedPackCacheUsage, GitStorageError> {
        let mut state = self.lock_state()?;
        let mut entries = cache_entries(&self.root, &state)?;
        entries.sort_by_key(|entry| entry.last_used);
        let mut usage = usage_for_entries(&entries, &state)?;
        for entry in entries {
            if usage.retained_bytes <= target_bytes {
                break;
            }
            if state
                .leases
                .get(&entry.pack_path)
                .copied()
                .unwrap_or_default()
                > 0
            {
                continue;
            }
            remove_cache_artifacts(&entry.pack_path)?;
            state.last_used.remove(&entry.pack_path);
            state.verified_at.remove(&entry.pack_path);
            usage.retained_bytes = usage.retained_bytes.saturating_sub(entry.size_bytes);
            if entry.has_pack {
                usage.pack_count = usage.pack_count.saturating_sub(1);
            }
        }
        usage.leased_bytes = cache_entries(&self.root, &state)?
            .into_iter()
            .filter(|entry| {
                state
                    .leases
                    .get(&entry.pack_path)
                    .copied()
                    .unwrap_or_default()
                    > 0
            })
            .try_fold(0_u64, |total, entry| total.checked_add(entry.size_bytes))
            .ok_or_else(cache_size_overflow)?;
        Ok(usage)
    }

    fn pin_locked(self: &Arc<Self>, state: &mut CacheState, path: PathBuf) -> VerifiedPackPin {
        *state.leases.entry(path.clone()).or_default() += 1;
        state.last_used.insert(path.clone(), SystemTime::now());
        VerifiedPackPin {
            cache: Arc::clone(self),
            path,
        }
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CacheState>, GitStorageError> {
        self.state
            .lock()
            .map_err(|_| GitStorageError::Task("verified Git pack cache lock is poisoned".into()))
    }
}

impl VerifiedPackFlight {
    pub(crate) fn complete(
        &self,
        result: Result<(GitSegmentRestoreTimings, VerifiedPackPin), GitStorageError>,
    ) {
        let outcome = match result {
            Ok((timings, pin)) => {
                if let Ok(mut publication_pin) = self.publication_pin.lock() {
                    *publication_pin = Some(pin);
                }
                Ok(timings)
            }
            Err(error) => Err(Arc::new(error)),
        };
        if let Ok(mut stored) = self.outcome.lock() {
            *stored = Some(outcome);
        }
        self.completed.notify_waiters();
    }

    pub(crate) async fn wait(&self) -> FlightOutcome {
        loop {
            let notified = self.completed.notified();
            match self.outcome.lock() {
                Ok(outcome) => {
                    if let Some(outcome) = outcome.as_ref() {
                        return outcome.clone();
                    }
                }
                Err(_) => {
                    return Err(Arc::new(GitStorageError::Task(
                        "verified Git pack flight lock is poisoned".into(),
                    )));
                }
            }
            notified.await;
        }
    }
}

impl Drop for VerifiedPackPin {
    fn drop(&mut self) {
        let Ok(mut state) = self.cache.state.lock() else {
            return;
        };
        match state.leases.get_mut(&self.path) {
            Some(count) if *count > 1 => *count -= 1,
            Some(_) => {
                state.leases.remove(&self.path);
            }
            None => {}
        }
        state.last_used.insert(self.path.clone(), SystemTime::now());
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
                .last_used
                .get(&pack_path)
                .copied()
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
        if state
            .leases
            .get(&entry.pack_path)
            .copied()
            .unwrap_or_default()
            > 0
        {
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
            state.verified_at.remove(path);
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
    } else if state.verified_at.get(path) != Some(&modified) {
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
        state.verified_at.remove(path);
        if state.leases.get(path).copied().unwrap_or_default() > 0 {
            return Err(error);
        }
        remove_cache_artifacts(path)?;
        state.last_used.remove(path);
        return Ok(false);
    }
    state.verified_at.insert(path.to_path_buf(), modified);
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
