mod config;
mod findings;
mod jsonc;
mod memory;
mod module_suffixes;
mod paths;
mod resolve;
mod scan;
mod snapshot;

use findings::{Findings, repository_gap};
use resolve::GroupContext;
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, DependencyGap,
};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    process::ExitCode,
};

pub const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
pub const ANALYZE_COMMAND: &str = "__analyze-dependencies";
const MAX_ANALYZER_MEMORY_BYTES: u64 = 512 * 1024 * 1024;
const ANALYZER_THREADS: usize = 2;

mod limits {
    pub(crate) const MAX_CONFIGS: usize = 64;
    pub(crate) const MAX_EDGES: usize = 25_000;
    pub(crate) const MAX_FILES: usize = 20_000;
    pub(crate) const MAX_GAPS: usize = 10_000;
    pub(crate) const MAX_PATH_BYTES: usize = 4_096;
    pub(crate) const MAX_SOURCE_BYTES: u64 = 2 * 1024 * 1024;
    pub(crate) const MAX_TOTAL_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
    pub(crate) const MAX_UNSUPPORTED_SOURCES: usize = 5_000;
    pub(crate) const MAX_REFERENCES: usize = 500_000;
    pub(crate) const MAX_CONCURRENT_SOURCE_BYTES: u64 = 8 * 1024 * 1024;
    pub(crate) const RESOLUTION_BATCH: usize = 16_384;
}

pub fn run_requested_analysis(arguments: impl Iterator<Item = OsString>) -> Option<ExitCode> {
    run_analysis_command(arguments, MAX_ANALYZER_MEMORY_BYTES)
}

fn run_analysis_command(
    mut arguments: impl Iterator<Item = OsString>,
    memory_limit_bytes: u64,
) -> Option<ExitCode> {
    let snapshot = match (arguments.nth(1), arguments.next(), arguments.next()) {
        (Some(command), Some(snapshot), None) if command == ANALYZE_COMMAND => {
            PathBuf::from(snapshot)
        }
        _ => return None,
    };
    if let Err(error) = memory::limit(memory_limit_bytes) {
        eprintln!("cannot limit dependency analyzer memory: {error}");
        return Some(ExitCode::FAILURE);
    }
    if let Err(error) = rayon::ThreadPoolBuilder::new()
        .num_threads(ANALYZER_THREADS)
        .build_global()
    {
        eprintln!("cannot limit dependency analyzer threads: {error}");
        return Some(ExitCode::FAILURE);
    }
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
    let mut findings = Findings::default();
    let scan = scan::scan_sources(inventory, &mut findings);
    let groups = match config::group_sources_by_config(&scan.resolvable_files, &inventory.configs) {
        Ok(groups) => groups,
        Err(reason) => {
            findings.add_repository_gap(reason);
            let (_, gaps) = findings.into_parts();
            return AnalyzerOutput {
                analyzed_files: scan.analyzed_files,
                unsupported_files: inventory.unsupported_files.clone(),
                gaps,
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
    for (config_path, sources) in &groups {
        let config = match config_path {
            Some(config_path) => match configs.resolution_config(config_path) {
                Ok(config) => Some(config),
                Err(reason) => {
                    findings.add_gap(DependencyGap {
                        path: config_path.clone(),
                        reason,
                    });
                    None
                }
            },
            None => None,
        };
        resolve::resolve_group(&context, config.as_ref(), sources, &mut findings);
    }

    let (edges, gaps) = findings.into_parts();
    AnalyzerOutput {
        analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
        analyzed_files: scan.analyzed_files,
        unsupported_files: inventory.unsupported_files.clone(),
        edges,
        gaps,
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
