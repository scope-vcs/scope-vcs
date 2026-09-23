use super::{
    BackendError, ENCODING_VERSION, GitSegmentRestoreSource, GitSegmentRestoreTimings,
    GitSegmentStore, GitStorageError, StagedGitSegment, VerifiedGitPack, is_hex_id_32,
    random_hex_id, segment_object_key, sync_directory,
};
use crate::{
    cache::{VerifiedPackCache, VerifiedPackFlight, VerifiedPackPin},
    envelope::{DecryptedFrame, EnvelopeReader, EnvelopeScope},
};
use scope_domain::repository::{RepositoryIncarnation, git::GitSegmentRef};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use tokio::{
    fs::{self, File, OpenOptions},
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
};

impl GitSegmentStore {
    pub async fn get_verified_pack(
        &self,
        incarnation: &RepositoryIncarnation,
        segment: &GitSegmentRef,
    ) -> Result<VerifiedGitPack, GitStorageError> {
        validate_restore_identity(incarnation.repository_id(), segment)?;
        let started = Instant::now();
        let path = self.verified_pack_path(incarnation, segment);
        if let Some(pack) = self
            .lease_verified_pack(
                &path,
                segment,
                local_timings(started, segment.plaintext_bytes),
            )
            .await?
        {
            return Ok(pack);
        }

        let (flight, starts_hydration) = self.verified_cache.begin_flight(&path);
        if starts_hydration {
            let store = self.clone();
            let incarnation = incarnation.clone();
            let segment = segment.clone();
            let guard = HydrationFlight {
                cache: Arc::clone(&self.verified_cache),
                path: path.clone(),
                flight: Arc::clone(&flight),
            };
            tokio::spawn(async move {
                let path = guard.path.clone();
                let result = store
                    .lead_hydration(&incarnation, &segment, path, started)
                    .await;
                guard.complete(result);
            });
        }

        let timings = flight
            .wait()
            .await
            .map_err(GitStorageError::VerifiedPackHydration)?;
        self.lease_verified_pack(&path, segment, timings)
            .await?
            .ok_or_else(|| {
                GitStorageError::Task(
                    "verified Git pack disappeared while hydration was leased".into(),
                )
            })
    }

    async fn lead_hydration(
        &self,
        incarnation: &RepositoryIncarnation,
        segment: &GitSegmentRef,
        path: PathBuf,
        started: Instant,
    ) -> Result<(GitSegmentRestoreTimings, VerifiedPackPin), GitStorageError> {
        let _permit = self.hydration_permits.acquire().await.map_err(|_| {
            GitStorageError::Task("verified Git pack hydration limit is unavailable".into())
        })?;
        let hydration_started = Instant::now();
        let plaintext_bytes = segment.plaintext_bytes;
        // A prior flight may have finished between our first miss
        // and registration. Recheck after becoming the leader.
        if let Some(pack) = self
            .lease_verified_pack(&path, segment, local_timings(started, plaintext_bytes))
            .await?
        {
            return Ok(pack.into_publication());
        }
        let repository_id = incarnation.repository_id();
        let incarnation_id = incarnation.incarnation_id();
        let segment_id = segment.segment_id.as_str();
        tracing::info!(
            repository_id,
            repository_incarnation_id = incarnation_id,
            segment_id,
            source = ?GitSegmentRestoreSource::Remote,
            bytes = plaintext_bytes,
            "verified Git pack hydration started"
        );
        let result = self.hydrate_verified_pack(incarnation, segment, path).await;
        tracing::info!(
            repository_id,
            repository_incarnation_id = incarnation_id,
            segment_id,
            source = ?GitSegmentRestoreSource::Remote,
            duration_us = hydration_started.elapsed().as_micros(),
            bytes = plaintext_bytes,
            success = result.is_ok(),
            "verified Git pack hydration completed"
        );
        result
    }

    async fn lease_verified_pack(
        &self,
        path: &Path,
        segment: &GitSegmentRef,
        timings: GitSegmentRestoreTimings,
    ) -> Result<Option<VerifiedGitPack>, GitStorageError> {
        let cache = Arc::clone(&self.verified_cache);
        let path = path.to_path_buf();
        let segment = segment.clone();
        tokio::task::spawn_blocking(move || {
            cache.lease_existing(&path, segment.plaintext_bytes, &segment.sha256, timings)
        })
        .await
        .map_err(|error| GitStorageError::Task(format!("leasing verified Git pack: {error}")))?
    }

    async fn install_verified_pack(
        &self,
        source: &Path,
        path: &Path,
        segment: &GitSegmentRef,
    ) -> Result<VerifiedPackPin, GitStorageError> {
        let cache = Arc::clone(&self.verified_cache);
        let source = source.to_path_buf();
        let path = path.to_path_buf();
        let segment = segment.clone();
        tokio::task::spawn_blocking(move || {
            cache.install(&source, &path, segment.plaintext_bytes, &segment.sha256)
        })
        .await
        .map_err(|error| GitStorageError::Task(format!("installing verified Git pack: {error}")))?
    }

    pub async fn promote_verified_pack(
        &self,
        incarnation: &RepositoryIncarnation,
        staged: &StagedGitSegment,
    ) -> Result<VerifiedGitPack, GitStorageError> {
        validate_restore_identity(incarnation.repository_id(), &staged.segment)?;
        if staged.object_key
            != segment_object_key(incarnation.repository_id(), &staged.segment.segment_id)
        {
            return Err(GitStorageError::InvalidConfiguration(
                "staged Git segment does not belong to this repository".into(),
            ));
        }
        let started = Instant::now();
        let path = self.verified_pack_path(incarnation, &staged.segment);
        let parent = verified_pack_parent(&path)?;
        fs::create_dir_all(&parent)
            .await
            .map_err(GitStorageError::Local)?;
        let pin = self
            .install_verified_pack(staged.local_pack_path(), &path, &staged.segment)
            .await?;
        sync_directory(parent)
            .await
            .map_err(GitStorageError::Local)?;
        Ok(VerifiedGitPack::new(
            path,
            local_timings(started, staged.segment.plaintext_bytes),
            pin,
        ))
    }

    pub async fn restore_to<W>(
        &self,
        repository_id: &str,
        segment: &GitSegmentRef,
        mut output: W,
    ) -> Result<GitSegmentRestoreTimings, GitStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        validate_restore_identity(repository_id, segment)?;
        let started = Instant::now();
        let object_key = segment_object_key(repository_id, &segment.segment_id);
        let mut source = self.backend.read(&object_key).await?;
        let mut envelope = EnvelopeReader::read_header(
            &mut source,
            &self.encryption_key,
            EnvelopeScope::git_segment(repository_id, &segment.segment_id),
        )
        .await?;
        let mut digest = Sha256::new();
        let mut plaintext_bytes = 0_u64;
        let mut frames = 0_u32;
        while let DecryptedFrame::Data(bytes) = envelope.next(&mut source).await? {
            output
                .write_all(&bytes)
                .await
                .map_err(GitStorageError::Output)?;
            digest.update(&bytes);
            plaintext_bytes = plaintext_bytes
                .checked_add(bytes.len() as u64)
                .ok_or_else(|| {
                    GitStorageError::InvalidEnvelope("plaintext size overflow".into())
                })?;
            frames = frames
                .checked_add(1)
                .ok_or_else(|| GitStorageError::InvalidEnvelope("frame count overflow".into()))?;
        }
        let mut trailing = [0_u8; 1];
        if source
            .read(&mut trailing)
            .await
            .map_err(|error| GitStorageError::Backend(BackendError::new(error.to_string())))?
            != 0
        {
            return Err(GitStorageError::InvalidEnvelope(
                "data follows the final frame".into(),
            ));
        }
        output.flush().await.map_err(GitStorageError::Output)?;
        verify_plaintext(segment, plaintext_bytes, digest)?;
        Ok(GitSegmentRestoreTimings {
            total: started.elapsed(),
            plaintext_bytes,
            verified_frames: frames,
            source: GitSegmentRestoreSource::Remote,
        })
    }

    pub async fn restore_to_prefer_local<W>(
        &self,
        repository_id: &str,
        segment: &GitSegmentRef,
        output: W,
    ) -> Result<GitSegmentRestoreTimings, GitStorageError>
    where
        W: AsyncWrite + Unpin + Send,
    {
        validate_restore_identity(repository_id, segment)?;
        let started = Instant::now();
        let local_path = self.local_pack_path(repository_id, &segment.segment_id);
        match File::open(local_path).await {
            Ok(file) => {
                restore_plaintext_local(file, output, segment, self.config.chunk_bytes, started)
                    .await
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let mut timings = self.restore_to(repository_id, segment, output).await?;
                timings.total = started.elapsed();
                Ok(timings)
            }
            Err(error) => Err(GitStorageError::Local(error)),
        }
    }

    async fn hydrate_verified_pack(
        &self,
        incarnation: &RepositoryIncarnation,
        segment: &GitSegmentRef,
        path: PathBuf,
    ) -> Result<(GitSegmentRestoreTimings, VerifiedPackPin), GitStorageError> {
        let parent = verified_pack_parent(&path)?;
        let temp_directory = self.verified_temp_directory();
        fs::create_dir_all(&parent)
            .await
            .map_err(GitStorageError::Local)?;
        fs::create_dir_all(&temp_directory)
            .await
            .map_err(GitStorageError::Local)?;
        let temp_path = temp_directory.join(format!(
            "{}.pack.tmp",
            random_hex_id().map_err(|error| GitStorageError::Task(format!(
                "creating verified pack temp path: {error}"
            )),)?
        ));
        let _temp = TemporaryPack::new(temp_path.clone());
        let mut output = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp_path)
            .await
            .map_err(GitStorageError::Local)?;
        let timings = self
            .restore_to(incarnation.repository_id(), segment, &mut output)
            .await?;
        output.sync_all().await.map_err(GitStorageError::Local)?;
        drop(output);
        let pin = self
            .install_verified_pack(&temp_path, &path, segment)
            .await?;
        sync_directory(parent)
            .await
            .map_err(GitStorageError::Local)?;
        Ok((timings, pin))
    }
}

/// Owns a hydration flight for the leader task. Dropping without `complete`
/// (a panic while hydrating) fails the flight so waiters do not hang, and
/// the flight is always unregistered exactly once.
struct HydrationFlight {
    cache: Arc<VerifiedPackCache>,
    path: PathBuf,
    flight: Arc<VerifiedPackFlight>,
}

impl HydrationFlight {
    fn complete(
        self,
        result: Result<(GitSegmentRestoreTimings, VerifiedPackPin), GitStorageError>,
    ) {
        self.flight.complete(result);
    }
}

impl Drop for HydrationFlight {
    fn drop(&mut self) {
        // No-op when `complete` already stored the real result.
        self.flight.complete(Err(GitStorageError::Task(
            "verified Git pack hydration task failed".into(),
        )));
        self.cache.finish_flight(&self.path, &self.flight);
    }
}

struct TemporaryPack {
    path: PathBuf,
}

impl TemporaryPack {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TemporaryPack {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn verified_pack_parent(path: &std::path::Path) -> Result<PathBuf, GitStorageError> {
    path.parent().map(PathBuf::from).ok_or_else(|| {
        GitStorageError::InvalidConfiguration("verified Git pack path has no parent".into())
    })
}

fn validate_restore_identity(
    repository_id: &str,
    segment: &GitSegmentRef,
) -> Result<(), GitStorageError> {
    if segment.encoding_version != ENCODING_VERSION {
        return Err(GitStorageError::InvalidEnvelope(format!(
            "unsupported encoding version {}",
            segment.encoding_version
        )));
    }
    if repository_id.is_empty() || !is_hex_id_32(&segment.segment_id) {
        return Err(GitStorageError::InvalidEnvelope(
            "repository id or segment id is invalid".into(),
        ));
    }
    Ok(())
}

async fn restore_plaintext_local<W>(
    mut source: File,
    mut output: W,
    segment: &GitSegmentRef,
    chunk_bytes: usize,
    started: Instant,
) -> Result<GitSegmentRestoreTimings, GitStorageError>
where
    W: AsyncWrite + Unpin + Send,
{
    let mut digest = Sha256::new();
    let mut plaintext_bytes = 0_u64;
    let mut buffer = vec![0_u8; chunk_bytes];
    loop {
        let read = source
            .read(&mut buffer)
            .await
            .map_err(GitStorageError::Local)?;
        if read == 0 {
            break;
        }
        output
            .write_all(&buffer[..read])
            .await
            .map_err(GitStorageError::Output)?;
        digest.update(&buffer[..read]);
        plaintext_bytes = plaintext_bytes
            .checked_add(read as u64)
            .ok_or_else(|| GitStorageError::InvalidEnvelope("plaintext size overflow".into()))?;
    }
    output.flush().await.map_err(GitStorageError::Output)?;
    verify_plaintext(segment, plaintext_bytes, digest)?;
    Ok(local_timings(started, plaintext_bytes))
}

fn local_timings(started: Instant, plaintext_bytes: u64) -> GitSegmentRestoreTimings {
    GitSegmentRestoreTimings {
        total: started.elapsed(),
        plaintext_bytes,
        verified_frames: 0,
        source: GitSegmentRestoreSource::Local,
    }
}

fn verify_plaintext(
    segment: &GitSegmentRef,
    plaintext_bytes: u64,
    digest: Sha256,
) -> Result<(), GitStorageError> {
    if plaintext_bytes != segment.plaintext_bytes {
        return Err(GitStorageError::SizeMismatch {
            expected: segment.plaintext_bytes,
            actual: plaintext_bytes,
        });
    }
    let actual = hex::encode(digest.finalize());
    if actual != segment.sha256 {
        return Err(GitStorageError::ChecksumMismatch {
            expected: segment.sha256.clone(),
            actual,
        });
    }
    Ok(())
}
