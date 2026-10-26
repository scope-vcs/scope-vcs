mod cache;
mod snapshot;

use crate::{
    git_repo::GitRepo,
    progress::{CancellationToken, run_cancellable},
};
use anyhow::{Context, bail};
use scope_dependency_analyzer::{ANALYZE_COMMAND, MAX_OUTPUT_BYTES};
use scope_domain::dependency_analysis::{AnalyzerOutput, StoredDependencyAnalysis};
use std::{
    path::Path,
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

const ANALYZER_TIMEOUT: Duration = Duration::from_secs(120);

pub struct AnalysisJob {
    receiver: Receiver<Result<StoredDependencyAnalysis, String>>,
    cancellation: CancellationToken,
    worker: Option<JoinHandle<()>>,
    completed: bool,
}

impl AnalysisJob {
    pub fn start(repo: &GitRepo, reviewed_head_oid: &str) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancellation = CancellationToken::new();
        let worker_cancellation = cancellation.clone();
        let root = repo.root.clone();
        let commit = reviewed_head_oid.to_owned();
        let worker = thread::spawn(move || {
            let result = analyze_cached(&root, &commit, &worker_cancellation, analyze_snapshot)
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(result);
        });
        Self {
            receiver,
            cancellation,
            worker: Some(worker),
            completed: false,
        }
    }

    pub fn try_result(&mut self) -> Option<Result<StoredDependencyAnalysis, String>> {
        if self.completed {
            return None;
        }
        match self.receiver.try_recv() {
            Ok(result) => {
                self.completed = true;
                Some(result)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.completed = true;
                Some(Err("Dependency analysis stopped before completing".into()))
            }
        }
    }
}

impl Drop for AnalysisJob {
    fn drop(&mut self) {
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn analyze_cached(
    repo: &Path,
    commit: &str,
    cancellation: &CancellationToken,
    analyze: impl FnOnce(&Path, &CancellationToken) -> anyhow::Result<AnalyzerOutput>,
) -> anyhow::Result<StoredDependencyAnalysis> {
    snapshot::validate_commit(commit)?;
    cancellation.check()?;
    let cache_path = cache::path(repo, commit, cancellation).ok();
    if let Some(analysis) = cache_path
        .as_deref()
        .and_then(|path| cache::read(path, commit))
    {
        return Ok(analysis);
    }
    let snapshot = snapshot::materialize(repo, commit, cancellation)?;
    cancellation.check()?;
    let output = analyze(snapshot.path(), cancellation)?;
    let analysis = StoredDependencyAnalysis::from_output(commit, output)?;
    cancellation.check()?;
    if let Some(path) = cache_path {
        let _ = cache::write(&path, &analysis);
    }
    Ok(analysis)
}

fn analyze_snapshot(
    snapshot: &Path,
    cancellation: &CancellationToken,
) -> anyhow::Result<AnalyzerOutput> {
    let executable = std::env::current_exe().context("Find the running Scope executable")?;
    analyze_snapshot_with(&executable, snapshot, cancellation)
}

pub fn analyze_snapshot_with(
    executable: &Path,
    snapshot: &Path,
    cancellation: &CancellationToken,
) -> anyhow::Result<AnalyzerOutput> {
    let mut command = Command::new(executable);
    command.env_clear();
    for key in ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(snapshot)
        .arg(ANALYZE_COMMAND)
        .arg(snapshot);
    let output = run_cancellable(
        &mut command,
        None,
        cancellation,
        ANALYZER_TIMEOUT,
        MAX_OUTPUT_BYTES,
    )
    .context("Check JS/TS imports")?;
    if !output.status.success() {
        bail!(
            "Dependency analyzer failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    serde_json::from_slice(&output.stdout).context("Read dependency analysis result")
}

#[cfg(test)]
mod tests;
