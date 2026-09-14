use scope_domain::{
    dependency_analysis::{
        DependencyReport, StoredDependencyAnalysis, evaluate_dependency_analysis,
    },
    repo_config::RepoConfig,
};

#[derive(Clone, Debug)]
pub(super) struct DependencyReview {
    expected_commit_oid: Option<String>,
    analysis: Option<StoredDependencyAnalysis>,
    status: DependencyStatus,
    expanded: bool,
}

#[derive(Clone, Debug)]
enum DependencyStatus {
    Hidden,
    Pending,
    Report(DependencyReport),
    Unsupported,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct DependencySummary {
    pub label: String,
    pub meta: Option<String>,
    pub expanded: bool,
    pub expandable: bool,
    pub warning: bool,
}

impl DependencyReview {
    pub fn hidden() -> Self {
        Self {
            expected_commit_oid: None,
            analysis: None,
            status: DependencyStatus::Hidden,
            expanded: false,
        }
    }

    pub fn pending(expected_commit_oid: impl Into<String>) -> Self {
        Self {
            expected_commit_oid: Some(expected_commit_oid.into()),
            analysis: None,
            status: DependencyStatus::Pending,
            expanded: false,
        }
    }

    pub fn is_visible(&self) -> bool {
        !matches!(self.status, DependencyStatus::Hidden)
    }

    pub fn complete(
        &mut self,
        result: Result<StoredDependencyAnalysis, String>,
        config: &RepoConfig,
    ) {
        let analysis = result.ok().filter(|analysis| {
            self.expected_commit_oid.as_deref() == Some(analysis.commit_oid.as_str())
        });
        let Some(analysis) = analysis else {
            self.analysis = None;
            self.status = DependencyStatus::Unavailable;
            self.expanded = false;
            return;
        };
        self.analysis = Some(analysis);
        self.reevaluate(config);
    }

    pub fn reevaluate(&mut self, config: &RepoConfig) {
        let Some(analysis) = &self.analysis else {
            return;
        };
        self.status = match evaluate_dependency_analysis(analysis, config) {
            Ok(report) if report.is_unsupported() => {
                self.expanded = false;
                DependencyStatus::Unsupported
            }
            Ok(report) => DependencyStatus::Report(report),
            Err(_) => {
                self.expanded = false;
                DependencyStatus::Unavailable
            }
        };
    }

    pub fn toggle_expanded(&mut self) {
        if self.report().is_some() {
            self.expanded = !self.expanded;
        }
    }

    pub fn expand(&mut self) {
        if self.report().is_some() {
            self.expanded = true;
        }
    }

    pub fn collapse(&mut self) -> bool {
        if self.expanded {
            self.expanded = false;
            true
        } else {
            false
        }
    }

    pub fn expanded(&self) -> bool {
        self.expanded
    }

    pub fn report(&self) -> Option<&DependencyReport> {
        match &self.status {
            DependencyStatus::Report(report) => Some(report),
            _ => None,
        }
    }

    pub fn summary(&self) -> Option<DependencySummary> {
        let (label, meta, expandable, warning) = match &self.status {
            DependencyStatus::Hidden => return None,
            DependencyStatus::Pending => (
                "Checking JS/TS imports…".to_string(),
                Some("P can continue push".to_string()),
                false,
                false,
            ),
            DependencyStatus::Unsupported => (
                "Dependency check does not support these source files yet".to_string(),
                None,
                false,
                false,
            ),
            DependencyStatus::Unavailable => (
                "Dependency check unavailable".to_string(),
                Some("P can continue push".to_string()),
                false,
                true,
            ),
            DependencyStatus::Report(report) => {
                let incomplete = !report.gaps.is_empty();
                let label = if report.findings.is_empty() {
                    if incomplete {
                        "Dependency check incomplete".to_string()
                    } else {
                        "No public → private imports found".to_string()
                    }
                } else {
                    format!(
                        "{} public {} {} private files",
                        report.public_file_count,
                        if report.public_file_count == 1 {
                            "file"
                        } else {
                            "files"
                        },
                        if report.public_file_count == 1 {
                            "imports"
                        } else {
                            "import"
                        },
                    )
                };
                let meta = if incomplete {
                    Some("Check incomplete".to_string())
                } else {
                    Some("JS/TS only".to_string())
                };
                (label, meta, true, !report.findings.is_empty() || incomplete)
            }
        };
        Some(DependencySummary {
            label,
            meta,
            expanded: self.expanded,
            expandable,
            warning,
        })
    }
}
