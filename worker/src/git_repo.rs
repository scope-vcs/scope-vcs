use crate::{duration_ms, elapsed_ms};
use scope_domain::repository::{RepositoryIncarnation, git_compaction::GitCompactionPlan};
use scope_git::GitStorageLimits;
use scope_git_process::{
    ProcessCancellation, ProcessError, ProcessLimits, StreamingProcessError,
    configure_process_group, run as run_process, run_with_stdin_reader, run_with_stdout,
};
use scope_storage::{
    GitSegmentReservation, GitSegmentRestoreSource, GitSegmentRestoreTimings, GitSegmentStore,
    StagedGitSegment,
};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

type ObjectId = [u8; 20];

#[cfg(all(test, target_os = "linux"))]
mod cancellation_tests;

pub(crate) struct CompactedPack {
    pub(crate) staged: StagedGitSegment,
    pub(crate) metrics: CompactionPackMetrics,
}

pub(crate) struct CompactionPackMetrics {
    pub(crate) source_span_count: usize,
    pub(crate) source_pack_bytes: usize,
    pub(crate) compacted_bytes: usize,
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
    segment_store: Arc<GitSegmentStore>,
    incarnation: &RepositoryIncarnation,
    plan: &GitCompactionPlan,
    reservation: GitSegmentReservation,
    storage_limits: GitStorageLimits,
    timeout: Duration,
    data_dir: PathBuf,
) -> anyhow::Result<CompactedPack> {
    let repository_id = incarnation.repository_id();
    let total_started = Instant::now();
    let repo = TemporaryGitRepo::new(&data_dir)?;
    let init_started = Instant::now();
    run_git(
        None,
        &["init", "--bare", repo.path.to_string_lossy().as_ref()],
        None,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    let init_ms = elapsed_ms(init_started);
    let mut download = Duration::ZERO;
    let mut index = Duration::ZERO;
    let mut source_pack_bytes = 0usize;
    let mut local_restore_count = 0usize;
    let mut remote_restore_count = 0usize;
    for span in plan.selected_spans() {
        if span.segment.plaintext_bytes
            > u64::try_from(storage_limits.max_object_bytes()).unwrap_or(u64::MAX)
        {
            return Err(anyhow::anyhow!(
                "Git segment {} exceeds the configured object byte limit",
                span.segment.segment_id
            ));
        }
        let index_started = Instant::now();
        let restore = index_verified_git_segment(
            Arc::clone(&segment_store),
            incarnation,
            &span.segment,
            &repo.path,
            timeout,
        )
        .await?;
        match restore.source {
            GitSegmentRestoreSource::Local => local_restore_count += 1,
            GitSegmentRestoreSource::Remote => remote_restore_count += 1,
        }
        download += restore.total;
        index += index_started.elapsed().saturating_sub(restore.total);
        let bytes = usize::try_from(span.segment.plaintext_bytes).unwrap_or(usize::MAX);
        source_pack_bytes = source_pack_bytes.saturating_add(bytes);
    }
    let enumerate_started = Instant::now();
    let object_ids = enumerate_object_ids(&repo.path, timeout, storage_limits.max_object_bytes())?;
    let enumerate_ms = elapsed_ms(enumerate_started);
    let pack_started = Instant::now();
    let staged = ingest_compacted_pack(
        Arc::clone(&segment_store),
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
    verify_object_set(
        &data_dir,
        staged.local_pack_path(),
        &object_ids,
        timeout,
        storage_limits.max_object_bytes(),
    )?;
    let verify_ms = elapsed_ms(verify_started);
    let compacted_bytes = usize::try_from(staged.segment.plaintext_bytes).unwrap_or(usize::MAX);
    Ok(CompactedPack {
        metrics: CompactionPackMetrics {
            source_span_count: plan.selected_spans().len(),
            source_pack_bytes,
            compacted_bytes,
            local_restore_count,
            remote_restore_count,
            init_ms,
            download_ms: duration_ms(download),
            index_ms: duration_ms(index),
            enumerate_ms,
            pack_ms,
            verify_ms,
            total_ms: elapsed_ms(total_started),
        },
        staged,
    })
}

async fn index_verified_git_segment(
    segment_store: Arc<GitSegmentStore>,
    incarnation: &RepositoryIncarnation,
    segment: &scope_domain::repository::git::GitSegmentRef,
    repo: &Path,
    timeout: Duration,
) -> anyhow::Result<GitSegmentRestoreTimings> {
    let pack = segment_store
        .get_verified_pack(incarnation, segment)
        .await
        .map_err(anyhow::Error::new)?;
    let timings = pack.timings().clone();
    let repo = repo.to_path_buf();
    tokio::task::spawn_blocking(move || index_pack_file(&repo, pack.path(), timeout, 64 * 1024))
        .await
        .map_err(|error| anyhow::anyhow!("Git index-pack task failed: {error}"))??;
    Ok(timings)
}

fn index_pack_file(
    repo: &Path,
    pack_path: &Path,
    timeout: Duration,
    max_stdout_bytes: usize,
) -> anyhow::Result<()> {
    let input = fs::File::open(pack_path)?;
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo)
        .args(["index-pack", "--stdin"]);
    let output = run_with_stdin_reader(
        &mut command,
        input,
        ProcessLimits::new(timeout).with_max_stdout_bytes(max_stdout_bytes),
        "git index-pack --stdin",
    )
    .map_err(anyhow::Error::new)?;
    if !output.status.success() {
        anyhow::bail!(
            "git index-pack --stdin failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
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
        hex::encode_to_slice(object_id, &mut input[start..]).expect("SHA-1 output size is fixed");
        input.push(b'\n');
    }
    input
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

pub(crate) async fn index_git_segment(
    segment_store: &GitSegmentStore,
    repository_id: &str,
    segment: &scope_domain::repository::git::GitSegmentRef,
    repo: &Path,
    timeout: Duration,
    cancellation: Option<&ProcessCancellation>,
) -> anyhow::Result<GitSegmentRestoreTimings> {
    let cancelled = || {
        anyhow::Error::new(ProcessError::Cancelled {
            action: "git index-pack --stdin".to_string(),
        })
    };
    if cancellation.is_some_and(ProcessCancellation::is_cancelled) {
        return Err(cancelled());
    }
    let mut command = tokio::process::Command::new("git");
    command
        .arg("--git-dir")
        .arg(repo)
        .args(["index-pack", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    configure_process_group(command.as_std_mut());
    let mut child = command.spawn()?;
    let process_id = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("opening git index-pack stderr"))?;
    let stdout_task = tokio::spawn(read_git_pipe(stdout, 64 * 1024));
    let stderr_task = tokio::spawn(read_git_pipe(stderr, 64 * 1024));
    let operation = async {
        let restore = segment_store.restore_to_prefer_local(repository_id, segment, stdin);
        let wait = child.wait();
        let (restore, status) = tokio::join!(restore, wait);
        Ok::<_, anyhow::Error>((restore?, status?))
    };
    let result = tokio::select! {
        biased;
        _ = async {
            match cancellation {
                Some(cancellation) => cancellation.cancelled().await,
                None => std::future::pending::<()>().await,
            }
        } => Err(cancelled()),
        result = tokio::time::timeout(timeout, operation) => match result {
            Ok(result) => result,
            Err(_) => Err(anyhow::Error::new(ProcessError::TimedOut {
                action: "git index-pack --stdin".to_string(),
                timeout_ms: timeout.as_millis(),
                diagnostic: String::new(),
            })),
        },
    };
    if result.is_err() {
        terminate_git_child(&mut child, process_id).await;
    }
    let stdout = stdout_task.await;
    let stderr = stderr_task.await;
    let (restore, status) = result?;
    let _stdout = stdout??;
    let stderr = stderr??;
    if !status.success() {
        anyhow::bail!(
            "git index-pack --stdin failed: {}",
            String::from_utf8_lossy(&stderr).trim()
        );
    }
    Ok(restore)
}

async fn read_git_pipe(
    pipe: impl tokio::io::AsyncRead + Unpin,
    max_bytes: usize,
) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .await?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::other(
            "Git process diagnostic output exceeded its byte limit",
        ));
    }
    Ok(bytes)
}

async fn terminate_git_child(child: &mut tokio::process::Child, process_id: Option<u32>) {
    if let Some(process_id) = process_id {
        scope_git_process::kill_process_group(process_id);
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
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
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests;
