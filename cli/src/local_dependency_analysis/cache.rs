use super::{MAX_OUTPUT_BYTES, snapshot};
use crate::progress::CancellationToken;
use anyhow::Context;
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, StoredDependencyAnalysis,
};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub(super) fn path(
    repo: &Path,
    commit: &str,
    cancellation: &CancellationToken,
) -> anyhow::Result<PathBuf> {
    let output = snapshot::git(
        repo,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        None,
        16 * 1024,
        cancellation,
    )?;
    let common = PathBuf::from(std::str::from_utf8(&output)?.trim());
    anyhow::ensure!(common.is_absolute(), "Git common directory is not absolute");
    let identity = format!("{commit}\0{DEPENDENCY_ANALYZER_VERSION}");
    let key = hex::encode(Sha256::digest(identity.as_bytes()));
    // Common Git storage is the repository identity and is shared by worktrees.
    Ok(common
        .join("scope/dependency-analysis")
        .join(format!("{key}.json")))
}

pub(super) fn read(path: &Path, commit: &str) -> Option<StoredDependencyAnalysis> {
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(MAX_OUTPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return None;
    }
    let cached: StoredDependencyAnalysis = serde_json::from_slice(&bytes).ok()?;
    if cached.commit_oid != commit || cached.analyzer_version != DEPENDENCY_ANALYZER_VERSION {
        return None;
    }
    StoredDependencyAnalysis::from_output(
        commit,
        AnalyzerOutput {
            analyzer_version: cached.analyzer_version,
            analyzed_files: cached.analyzed_files,
            unsupported_files: cached.unsupported_files,
            edges: cached.edges,
            gaps: cached.gaps,
        },
    )
    .ok()
}

pub(super) fn write(path: &Path, analysis: &StoredDependencyAnalysis) -> anyhow::Result<()> {
    let parent = path.parent().context("Dependency cache has no directory")?;
    fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut file, analysis)?;
    file.flush()?;
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}
