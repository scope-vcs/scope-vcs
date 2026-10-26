mod config;
mod jsonc;
mod module_suffixes;
mod paths;
mod resolve;
mod scan;
mod snapshot;

use resolve::{GroupContext, kind_name};
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, DependencyEdge, DependencyGap,
};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const ANALYZE_COMMAND: &str = "__analyze-dependencies";

mod limits {
    pub(crate) const MAX_CONFIGS: usize = 64;
    pub(crate) const MAX_EDGES: usize = 25_000;
    pub(crate) const MAX_FILES: usize = 20_000;
    pub(crate) const MAX_GAPS: usize = 10_000;
    pub(crate) const MAX_PATH_BYTES: usize = 4_096;
    pub(crate) const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
    pub(crate) const MAX_TOTAL_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
    pub(crate) const MAX_UNSUPPORTED_SOURCES: usize = 5_000;
}

pub fn run_requested_analysis(mut arguments: impl Iterator<Item = OsString>) -> Option<ExitCode> {
    let snapshot = match (arguments.nth(1), arguments.next(), arguments.next()) {
        (Some(command), Some(snapshot), None) if command == ANALYZE_COMMAND => {
            PathBuf::from(snapshot)
        }
        _ => return None,
    };
    let output = analyze_snapshot(&snapshot);
    let written = serde_json::to_writer(std::io::stdout().lock(), &output)
        .map_err(std::io::Error::from)
        .and_then(|()| std::io::stdout().flush());
    Some(if written.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

pub fn analyze_snapshot(snapshot: &Path) -> AnalyzerOutput {
    let output = match snapshot::inventory(snapshot) {
        Ok(inventory) => analyze_inventory(&inventory),
        Err(error) => incomplete(error.0),
    };
    within_output_budget(output)
}

fn analyze_inventory(inventory: &snapshot::Inventory) -> AnalyzerOutput {
    let scan = scan::scan_sources(inventory);
    let mut gaps = scan.gaps;
    let groups = match config::group_sources_by_config(&scan.resolvable_files, &inventory.configs) {
        Ok(groups) => groups,
        Err(reason) => {
            gaps.push(repository_gap(reason));
            return AnalyzerOutput {
                analyzed_files: scan.analyzed_files,
                unsupported_files: inventory.unsupported_files.clone(),
                gaps: bounded_gaps(gaps),
                ..incomplete_base()
            };
        }
    };

    let internal_package_names = inventory.internal_package_names();
    let context = GroupContext {
        inventory,
        internal_package_names: &internal_package_names,
        references: &scan.references,
    };
    let mut configs = config::ConfigLoader::new(inventory);
    let mut edges = Vec::new();
    for (config_path, sources) in &groups {
        let config = match config_path {
            Some(config_path) => match configs.resolution_config(config_path) {
                Ok(config) => Some(config),
                Err(reason) => {
                    gaps.push(DependencyGap {
                        path: config_path.clone(),
                        reason,
                    });
                    None
                }
            },
            None => None,
        };
        let group = resolve::resolve_group(&context, config.as_ref(), sources);
        edges.extend(group.edges);
        gaps.extend(group.gaps);
    }

    edges.sort_by(|left, right| edge_key(left).cmp(&edge_key(right)));
    edges.dedup();
    if edges.len() > limits::MAX_EDGES {
        gaps.push(repository_gap(format!(
            "dependency edge limit exceeded ({})",
            limits::MAX_EDGES
        )));
        edges.clear();
    }

    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
        analyzed_files: scan.analyzed_files,
        unsupported_files: inventory.unsupported_files.clone(),
        edges,
        gaps: bounded_gaps(gaps),
    }
}

fn edge_key(edge: &DependencyEdge) -> (&str, &str, &str) {
    (&edge.source_path, &edge.target_path, kind_name(edge.kind))
}

fn bounded_gaps(mut gaps: Vec<DependencyGap>) -> Vec<DependencyGap> {
    gaps.sort();
    gaps.dedup();
    if gaps.len() > limits::MAX_GAPS {
        gaps.truncate(limits::MAX_GAPS - 1);
        gaps.push(repository_gap(format!(
            "coverage gap limit exceeded ({})",
            limits::MAX_GAPS
        )));
    }
    gaps
}

fn repository_gap(reason: String) -> DependencyGap {
    DependencyGap {
        path: ".".into(),
        reason,
    }
}

fn incomplete_base() -> AnalyzerOutput {
    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
        analyzed_files: Vec::new(),
        unsupported_files: Vec::new(),
        edges: Vec::new(),
        gaps: Vec::new(),
    }
}

fn incomplete(reason: String) -> AnalyzerOutput {
    AnalyzerOutput {
        gaps: vec![repository_gap(reason)],
        ..incomplete_base()
    }
}

fn within_output_budget(output: AnalyzerOutput) -> AnalyzerOutput {
    if serialized_len(&output) <= MAX_OUTPUT_BYTES {
        return output;
    }
    incomplete("Dependency results exceed the 8 MiB limit; analysis is incomplete.".into())
}

fn serialized_len(output: &AnalyzerOutput) -> usize {
    serde_json::to_vec(output).map_or(usize::MAX, |bytes| bytes.len())
}

#[cfg(test)]
mod tests;
