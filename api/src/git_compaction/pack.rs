use super::elapsed_ms;
use crate::{
    error::ApiError,
    git::pool_install::{FetchedPack, fetch_pack, install_fetched},
    state::AppState,
};
use scope_domain::repository::{RepositoryIncarnation, git_compaction::GitCompactionPlan};
use scope_git_process::{
    ProcessCancellation, ProcessError, ProcessLimits, StreamingProcessError, run as run_process,
    run_with_stdin_reader, run_with_stdout,
};
use scope_storage::{
    GitSegmentReservation, GitSegmentRestoreSource, GitSegmentStore, StagedGitSegment,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};

type ObjectId = [u8; 20];

pub(crate) struct CompactedPack {
    pub(crate) staged: StagedGitSegment,
    pub(crate) metrics: CompactionPackMetrics,
}

pub(crate) struct CompactionPackMetrics {
    pub(crate) source_pack_count: usize,
    pub(crate) source_pack_bytes: usize,
    pub(crate) compacted_bytes: usize,
    pub(crate) inline_source_count: usize,
    pub(crate) local_restore_count: usize,
    pub(crate) remote_restore_count: usize,
    pub(crate) init_ms: u64,
    pub(crate) download_ms: u64,
    pub(crate) index_ms: u64,
    pub(crate) enumerate_ms: u64,
    pub(crate) pack_ms: u64,
    pub(crate) verify_ms: u64,
    pub(crate) total_ms: u64,
}

pub(crate) async fn build_compacted_pack(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    plan: &GitCompactionPlan,
    reservation: GitSegmentReservation,
    timeout: Duration,
) -> anyhow::Result<CompactedPack> {
    let storage_limits = state.runtime_budgets.git_storage_limits();
    let data_dir = state.data_dir.as_ref();
    let repository_id = incarnation.repository_id();
    let total_started = Instant::now();
    let repo = TemporaryGitRepo::new(data_dir)?;
    let max_object_bytes = storage_limits.max_object_bytes();
    let init_started = Instant::now();
    let repo_path = repo.path.clone();
    blocking(move || {
        run_git(
            None,
            &["init", "--bare", repo_path.to_string_lossy().as_ref()],
            None,
            timeout,
            max_object_bytes,
        )
        .map(drop)
    })
    .await?;
    let init_ms = elapsed_ms(init_started);
    let mut download = Duration::ZERO;
    let mut index = Duration::ZERO;
    let mut source_pack_bytes = 0usize;
    let mut inline_source_count = 0usize;
    let mut local_restore_count = 0usize;
    let mut remote_restore_count = 0usize;
    for pack in plan.selected_packs() {
        let bytes = pack.storage.stored_bytes();
        if bytes > u64::try_from(storage_limits.max_object_bytes()).unwrap_or(u64::MAX) {
            return Err(anyhow::anyhow!(
                "Git pool pack {} exceeds the configured object byte limit",
                pack.id
            ));
        }
        let fetch_started = Instant::now();
        let fetched = fetch_pack(&state.git_segment_store, incarnation, pack)
            .await
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        match &fetched {
            FetchedPack::Verified(verified) => {
                match verified.timings().source {
                    GitSegmentRestoreSource::Local => local_restore_count += 1,
                    GitSegmentRestoreSource::Remote => remote_restore_count += 1,
                }
                download += fetch_started.elapsed();
            }
            FetchedPack::Bytes(_) => inline_source_count += 1,
        }
        let index_started = Instant::now();
        install_fetched(&repo.path, fetched, timeout, &ProcessCancellation::new())
            .await
            .map_err(|error| install_failure(error, index_started.elapsed(), timeout))?;
        index += index_started.elapsed();
        source_pack_bytes =
            source_pack_bytes.saturating_add(usize::try_from(bytes).unwrap_or(usize::MAX));
    }
    let enumerate_started = Instant::now();
    let repo_path = repo.path.clone();
    let object_ids =
        blocking(move || enumerate_object_ids(&repo_path, timeout, max_object_bytes)).await?;
    let enumerate_ms = elapsed_ms(enumerate_started);
    let pack_started = Instant::now();
    let staged = ingest_compacted_pack(
        Arc::clone(&state.git_segment_store),
        repository_id,
        reservation,
        &repo.path,
        object_id_input(&object_ids),
        timeout,
        storage_limits.max_object_bytes(),
    )
    .await?;
    let pack_ms = elapsed_ms(pack_started);
    let verify_started = Instant::now();
    let verify_data_dir = data_dir.to_path_buf();
    let verify_pack = staged.local_pack_path().to_path_buf();
    blocking(move || {
        verify_object_set(
            &verify_data_dir,
            &verify_pack,
            &object_ids,
            timeout,
            max_object_bytes,
        )
    })
    .await?;
    let verify_ms = elapsed_ms(verify_started);
    let compacted_bytes = usize::try_from(staged.segment.plaintext_bytes).unwrap_or(usize::MAX);
    Ok(CompactedPack {
        metrics: CompactionPackMetrics {
            source_pack_count: plan.selected_packs().len(),
            source_pack_bytes,
            compacted_bytes,
            inline_source_count,
            local_restore_count,
            remote_restore_count,
            init_ms,
            download_ms: duration_millis(download),
            index_ms: duration_millis(index),
            enumerate_ms,
            pack_ms,
            verify_ms,
            total_ms: elapsed_ms(total_started),
        },
        staged,
    })
}

fn install_failure(error: ApiError, elapsed: Duration, timeout: Duration) -> anyhow::Error {
    let diagnostic = error.into_operator_diagnostic();
    if elapsed < timeout {
        return anyhow::anyhow!(diagnostic);
    }
    anyhow::Error::new(ProcessError::TimedOut {
        action: "installing a Git pool pack".to_string(),
        timeout_ms: timeout.as_millis(),
        diagnostic,
    })
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> anyhow::Result<T> + Send + 'static,
) -> anyhow::Result<T> {
    let span = tracing::Span::current();
    tokio::task::spawn_blocking(move || span.in_scope(work))
        .await
        .map_err(|error| anyhow::anyhow!("Git compaction task failed: {error}"))?
}

fn enumerate_object_ids(
    repo: &Path,
    timeout: Duration,
    max_bytes: usize,
) -> anyhow::Result<BTreeSet<ObjectId>> {
    let output = run_git(
        Some(repo),
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
        None,
        timeout,
        max_bytes,
    )?;
    let output = String::from_utf8(output)
        .map_err(|_| anyhow::anyhow!("Git compaction object enumeration was not UTF-8"))?;
    let mut object_ids = BTreeSet::new();
    for object_id in output.lines() {
        object_ids
            .insert(parse_object_id(object_id).ok_or_else(|| {
                anyhow::anyhow!("Git compaction enumerated an invalid object ID")
            })?);
    }
    if object_ids.is_empty() {
        anyhow::bail!("Git compaction selected packs contain no objects");
    }
    Ok(object_ids)
}

fn parse_object_id(value: &str) -> Option<ObjectId> {
    let mut object_id = [0_u8; 20];
    (value.len() == 40 && hex::decode_to_slice(value, &mut object_id).is_ok()).then_some(object_id)
}

fn object_id_input(object_ids: &BTreeSet<ObjectId>) -> Vec<u8> {
    let mut input = Vec::with_capacity(object_ids.len().saturating_mul(41));
    for object_id in object_ids {
        let start = input.len();
        input.resize(start + 40, 0);
        #[expect(clippy::expect_used)]
        hex::encode_to_slice(object_id, &mut input[start..])
            .expect("a 20-byte object id encodes into exactly 40 hex bytes");
        input.push(b'\n');
    }
    input
}

fn index_pack_file(
    repo: &Path,
    pack_path: &Path,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<()> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo)
        .args(["index-pack", "--stdin"]);
    let output = run_with_stdin_reader(
        &mut command,
        fs::File::open(pack_path)?,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        "git index-pack --stdin",
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "git index-pack --stdin failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn verify_object_set(
    data_dir: &Path,
    pack_path: &Path,
    expected: &BTreeSet<ObjectId>,
    timeout: Duration,
    max_bytes: usize,
) -> anyhow::Result<()> {
    let repo = TemporaryGitRepo::new(data_dir)?;
    run_git(
        None,
        &["init", "--bare", repo.path.to_string_lossy().as_ref()],
        None,
        timeout,
        max_bytes,
    )?;
    index_pack_file(&repo.path, pack_path, timeout, max_bytes)?;
    let actual = enumerate_object_ids(&repo.path, timeout, max_bytes)?;
    if &actual != expected {
        anyhow::bail!(
            "Git compaction replacement object set differs from its selected packs (expected {}, found {})",
            expected.len(),
            actual.len()
        );
    }
    Ok(())
}

async fn ingest_compacted_pack(
    segment_store: Arc<GitSegmentStore>,
    repository_id: &str,
    reservation: GitSegmentReservation,
    repo: &Path,
    object_ids: Vec<u8>,
    timeout: Duration,
    max_bytes: usize,
) -> anyhow::Result<StagedGitSegment> {
    let repository_id = repository_id.to_string();
    let repo = repo.to_path_buf();
    let runtime = tokio::runtime::Handle::current();
    let span = tracing::Span::current();
    let output = tokio::task::spawn_blocking(move || {
        let mut command = Command::new("git");
        command
            .arg("--git-dir")
            .arg(repo)
            .args(["pack-objects", "--stdout"]);
        run_with_stdout(
            &mut command,
            Some(object_ids),
            ProcessLimits::new(timeout),
            "git pack-objects --stdout",
            move |stdout, cancellation| {
                let _entered = span.enter();
                runtime.block_on(segment_store.ingest_reserved_blocking_reader(
                    &repository_id,
                    reservation,
                    stdout,
                    max_bytes as u64,
                    Some(cancellation),
                ))
            },
        )
    })
    .await
    .map_err(|error| anyhow::anyhow!("Git compaction task failed: {error}"))?;
    let output = match output {
        Ok(output) => output,
        Err(StreamingProcessError::Process(error)) => {
            return Err(anyhow::Error::new(error));
        }
        Err(StreamingProcessError::Consumer(
            scope_storage::GitStorageError::PlaintextLimitExceeded { .. },
        )) => {
            return Err(anyhow::Error::new(ProcessError::StdoutLimitExceeded {
                action: "git pack-objects --stdout".to_string(),
                max_stdout_bytes: max_bytes,
                diagnostic: String::new(),
            }));
        }
        Err(StreamingProcessError::Consumer(error)) => {
            return Err(anyhow::Error::new(error));
        }
    };
    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "git pack-objects --stdout failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(output.value)
}

fn run_git(
    git_dir: Option<&Path>,
    args: &[&str],
    input: Option<Vec<u8>>,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    if let Some(git_dir) = git_dir {
        command.arg("--git-dir").arg(git_dir);
    }
    command.args(args);
    let output = run_process(
        &mut command,
        input,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        &format!("git {}", args.join(" ")),
    )
    .map_err(anyhow::Error::from)?;
    if !output.status.success() {
        anyhow::bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

struct TemporaryGitRepo {
    path: PathBuf,
}

impl TemporaryGitRepo {
    fn new(data_dir: &Path) -> anyhow::Result<Self> {
        let mut random = [0_u8; 16];
        getrandom::fill(&mut random)
            .map_err(|error| anyhow::anyhow!("creating compaction path: {error}"))?;
        let root = data_dir.join("git-compaction");
        fs::create_dir_all(&root)?;
        let path = root.join(format!(
            "scope-git-compact-{}-{}",
            std::process::id(),
            hex::encode(random)
        ));
        Ok(Self { path })
    }
}

impl Drop for TemporaryGitRepo {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.path)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(%error, path = %self.path.display(), "failed to remove temporary Git repository");
        }
    }
}

fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "pack_tests.rs"]
mod tests;
