//! Commit-scoped local import analysis. Visibility remains a domain evaluation.
mod cache;
mod snapshot;

use crate::{
    git_repo::GitRepo,
    progress::{CancellationToken, run_cancellable},
};
use anyhow::{Context, bail};
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, StoredDependencyAnalysis,
};
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread::{self, JoinHandle},
    time::Duration,
};

const ANALYZER_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

/// Owns background analysis and its subprocesses for one open review.
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
    check_cancelled(cancellation)?;
    // An unavailable cache never makes the actual check unavailable.
    let cache_path = cache::path(repo, commit, cancellation).ok();
    if let Some(analysis) = cache_path
        .as_deref()
        .and_then(|path| cache::read(path, commit))
    {
        return Ok(analysis);
    }
    let snapshot = snapshot::materialize(repo, commit, cancellation)?;
    check_cancelled(cancellation)?;
    let output = analyze(snapshot.path(), cancellation)?;
    if output.analyzer_version != DEPENDENCY_ANALYZER_VERSION {
        bail!("Installed dependency analyzer does not match this CLI; reinstall Scope");
    }
    let analysis = StoredDependencyAnalysis::from_output(commit, output)?;
    check_cancelled(cancellation)?;
    if let Some(path) = cache_path {
        let _ = cache::write(&path, &analysis);
    }
    Ok(analysis)
}

fn analyze_snapshot(
    snapshot: &Path,
    cancellation: &CancellationToken,
) -> anyhow::Result<AnalyzerOutput> {
    let runtime = runtime_directory()?;
    let node = runtime.join(if cfg!(windows) { "node.exe" } else { "node" });
    let analyzer = runtime.join("dependency-analyzer/analyze.mjs");
    if !node.is_file() || !analyzer.is_file() {
        bail!("Dependency analyzer is not installed with this CLI; reinstall Scope");
    }
    let mut command = Command::new(node);
    // Repository code and the caller's NODE_OPTIONS must not change execution.
    command.env_clear();
    for key in ["SystemRoot", "WINDIR", "TEMP", "TMP", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(snapshot)
        .arg("--max-old-space-size=512")
        .arg(analyzer)
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

fn runtime_directory() -> anyhow::Result<PathBuf> {
    // Explicit development override uses the same managed bundle layout.
    if let Some(path) = std::env::var_os("SCOPE_CLI_RUNTIME_DIR") {
        return PathBuf::from(path)
            .canonicalize()
            .context("Find dependency analyzer runtime");
    }
    let executable = std::env::current_exe().context("Find the running Scope executable")?;
    let parent = executable
        .parent()
        .context("Scope executable has no parent directory")?;
    Ok(parent.join("scope-runtime"))
}

fn check_cancelled(cancellation: &CancellationToken) -> anyhow::Result<()> {
    if cancellation.is_cancelled() {
        bail!("Dependency check canceled");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
