use crate::{
    policy::{ScopePath, Visibility},
    repo_config::RepoConfig,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use thiserror::Error;

pub const DEPENDENCY_ANALYZER_VERSION: &str = "dependency-cruiser@18.2.0+scope-1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnalyzerOutput {
    pub analyzer_version: String,
    pub analyzed_files: Vec<String>,
    pub unsupported_files: Vec<String>,
    pub edges: Vec<DependencyEdge>,
    pub gaps: Vec<DependencyGap>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DependencyEdge {
    pub source_path: String,
    pub target_path: String,
    pub kind: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DependencyGap {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredDependencyAnalysis {
    pub commit_oid: String,
    pub analyzer_version: String,
    pub analyzed_files: Vec<String>,
    pub unsupported_files: Vec<String>,
    pub edges: Vec<DependencyEdge>,
    pub gaps: Vec<DependencyGap>,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct DependencyFinding {
    pub source_path: String,
    pub target_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyReport {
    pub commit_oid: String,
    pub analyzer_version: String,
    pub analyzed_file_count: usize,
    pub unsupported_files: Vec<String>,
    pub gaps: Vec<DependencyGap>,
    pub findings: Vec<DependencyFinding>,
    pub public_file_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyCheckStatus {
    Pending,
    Ready,
    Updating,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DependencyCheck {
    pub status: DependencyCheckStatus,
    pub report: Option<DependencyReport>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DependencyAnalysisError {
    #[error("dependency analysis commit OID is required")]
    MissingCommit,
    #[error("dependency analyzer version is required")]
    MissingAnalyzerVersion,
    #[error("dependency path must be normalized and repository-relative: {0}")]
    InvalidPath(String),
    #[error("dependency edge kind is required")]
    MissingEdgeKind,
    #[error("dependency coverage gap reason is required")]
    MissingGapReason,
}

impl StoredDependencyAnalysis {
    pub fn from_output(
        commit_oid: impl Into<String>,
        output: AnalyzerOutput,
    ) -> Result<Self, DependencyAnalysisError> {
        let commit_oid = commit_oid.into();
        if commit_oid.trim().is_empty() {
            return Err(DependencyAnalysisError::MissingCommit);
        }
        if output.analyzer_version.trim().is_empty() {
            return Err(DependencyAnalysisError::MissingAnalyzerVersion);
        }

        let analyzed_files = normalized_paths(output.analyzed_files)?;
        let unsupported_files = normalized_paths(output.unsupported_files)?;
        let mut edges = output.edges;
        for edge in &edges {
            dependency_scope_path(&edge.source_path)?;
            dependency_scope_path(&edge.target_path)?;
            if edge.kind.trim().is_empty() {
                return Err(DependencyAnalysisError::MissingEdgeKind);
            }
        }
        edges.sort();
        edges.dedup();

        let mut gaps = output.gaps;
        for gap in &gaps {
            if gap.path != "." {
                dependency_scope_path(&gap.path)?;
            }
            if gap.reason.trim().is_empty() {
                return Err(DependencyAnalysisError::MissingGapReason);
            }
        }
        gaps.sort();
        gaps.dedup();

        Ok(Self {
            commit_oid,
            analyzer_version: output.analyzer_version,
            analyzed_files,
            unsupported_files,
            edges,
            gaps,
        })
    }
}

pub fn evaluate_dependency_analysis(
    analysis: &StoredDependencyAnalysis,
    config: &RepoConfig,
) -> Result<DependencyReport, DependencyAnalysisError> {
    let mut findings = BTreeSet::new();
    for edge in &analysis.edges {
        let source = dependency_scope_path(&edge.source_path)?;
        let target = dependency_scope_path(&edge.target_path)?;
        if config.visibility_for_path(&source) == Visibility::Public
            && config.visibility_for_path(&target) == Visibility::Private
        {
            findings.insert(DependencyFinding {
                source_path: edge.source_path.clone(),
                target_path: edge.target_path.clone(),
            });
        }
    }
    let findings = findings.into_iter().collect::<Vec<_>>();
    let public_file_count = findings
        .iter()
        .map(|finding| finding.source_path.as_str())
        .collect::<BTreeSet<_>>()
        .len();

    Ok(DependencyReport {
        commit_oid: analysis.commit_oid.clone(),
        analyzer_version: analysis.analyzer_version.clone(),
        analyzed_file_count: analysis.analyzed_files.len(),
        unsupported_files: analysis.unsupported_files.clone(),
        gaps: analysis.gaps.clone(),
        findings,
        public_file_count,
    })
}

fn normalized_paths(paths: Vec<String>) -> Result<Vec<String>, DependencyAnalysisError> {
    let paths = paths.into_iter().collect::<BTreeSet<_>>();
    for path in &paths {
        dependency_scope_path(path)?;
    }
    Ok(paths.into_iter().collect())
}

fn dependency_scope_path(path: &str) -> Result<ScopePath, DependencyAnalysisError> {
    if path.is_empty() || path.starts_with('/') || path.contains('\\') || path.ends_with('/') {
        return Err(DependencyAnalysisError::InvalidPath(path.to_string()));
    }
    let scope_path = ScopePath::parse(format!("/{path}"))
        .map_err(|_| DependencyAnalysisError::InvalidPath(path.to_string()))?;
    if scope_path.as_str().strip_prefix('/') != Some(path) {
        return Err(DependencyAnalysisError::InvalidPath(path.to_string()));
    }
    Ok(scope_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_config::{ConfigVisibility, RepoConfigVisibilityRule};

    fn config() -> RepoConfig {
        let mut config = RepoConfig::with_default_visibility(ConfigVisibility::Public);
        config.visibility.rules.push(RepoConfigVisibilityRule {
            path: "/private/**".into(),
            visibility: ConfigVisibility::Private,
        });
        config
    }

    fn analysis(edges: Vec<DependencyEdge>) -> StoredDependencyAnalysis {
        StoredDependencyAnalysis::from_output(
            "a".repeat(40),
            AnalyzerOutput {
                analyzer_version: DEPENDENCY_ANALYZER_VERSION.into(),
                analyzed_files: vec![
                    "public/a.ts".into(),
                    "public/b.ts".into(),
                    "private/c.ts".into(),
                    "private/d.ts".into(),
                ],
                unsupported_files: vec!["src/main.rs".into()],
                edges,
                gaps: vec![DependencyGap {
                    path: ".".into(),
                    reason: "snapshot inventory incomplete".into(),
                }],
            },
        )
        .unwrap()
    }

    fn edge(source: &str, target: &str, kind: &str) -> DependencyEdge {
        DependencyEdge {
            source_path: source.into(),
            target_path: target.into(),
            kind: kind.into(),
        }
    }

    #[test]
    fn reports_only_direct_public_to_private_pairs() {
        let report = evaluate_dependency_analysis(
            &analysis(vec![
                edge("public/a.ts", "public/b.ts", "import"),
                edge("public/b.ts", "private/c.ts", "import"),
                edge("private/c.ts", "private/d.ts", "import"),
            ]),
            &config(),
        )
        .unwrap();

        assert_eq!(
            report.findings,
            vec![DependencyFinding {
                source_path: "public/b.ts".into(),
                target_path: "private/c.ts".into(),
            }]
        );
        assert_eq!(report.public_file_count, 1);
        assert_eq!(report.gaps[0].path, ".");
    }

    #[test]
    fn retains_edge_kinds_but_deduplicates_report_pairs_and_source_count() {
        let analysis = analysis(vec![
            edge("public/a.ts", "private/c.ts", "import"),
            edge("public/a.ts", "private/c.ts", "type-import"),
            edge("public/a.ts", "private/d.ts", "re-export"),
        ]);
        assert_eq!(analysis.edges.len(), 3);

        let report = evaluate_dependency_analysis(&analysis, &config()).unwrap();
        assert_eq!(report.findings.len(), 2);
        assert_eq!(report.public_file_count, 1);
    }

    #[test]
    fn rejects_non_normalized_or_absolute_analyzer_paths() {
        for path in ["/src/a.ts", "src//a.ts", "src/../a.ts", "src\\a.ts", ""] {
            let error = StoredDependencyAnalysis::from_output(
                "head",
                AnalyzerOutput {
                    analyzer_version: "reader".into(),
                    analyzed_files: vec![path.into()],
                    unsupported_files: Vec::new(),
                    edges: Vec::new(),
                    gaps: Vec::new(),
                },
            )
            .unwrap_err();
            assert_eq!(error, DependencyAnalysisError::InvalidPath(path.into()));
        }
    }
}
