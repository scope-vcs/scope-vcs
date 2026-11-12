use scope_dependency_analyzer::analyze_snapshot;
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DependencyEdge, DependencyEdgeKind, DependencyGap,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

pub(crate) fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

pub(crate) fn analyze(root: &Path) -> AnalyzerOutput {
    analyze_snapshot(root)
}

pub(crate) fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

pub(crate) fn repository(files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().unwrap();
    write_files(root.path(), files);
    root
}

pub(crate) fn edge(
    source_path: &str,
    target_path: &str,
    kind: DependencyEdgeKind,
) -> DependencyEdge {
    DependencyEdge {
        source_path: source_path.into(),
        target_path: target_path.into(),
        kind,
    }
}

pub(crate) fn gap(path: &str, reason: &str) -> DependencyGap {
    DependencyGap {
        path: path.into(),
        reason: reason.into(),
    }
}

pub(crate) fn targets_from(output: &AnalyzerOutput, source: &str) -> Vec<String> {
    output
        .edges
        .iter()
        .filter(|edge| edge.source_path == source)
        .map(|edge| edge.target_path.clone())
        .collect()
}

pub(crate) fn has_gap(output: &AnalyzerOutput, path: &str, reason: impl Fn(&str) -> bool) -> bool {
    output
        .gaps
        .iter()
        .any(|gap| gap.path == path && reason(&gap.reason))
}
