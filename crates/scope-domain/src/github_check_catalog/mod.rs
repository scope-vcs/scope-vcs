mod branch_filter;
mod jobs;
mod triggers;

use crate::requests::GitHubBranch;
use std::collections::BTreeMap;

pub const GITHUB_WORKFLOW_DISABLED_MESSAGE: &str = "Disabled on GitHub.";
pub const GITHUB_PATH_FILTER_MESSAGE: &str =
    "Runs only when the changed files match its path filters.";
pub const GITHUB_JOB_CONDITION_MESSAGE: &str = "Runs only when its condition holds.";
pub const GITHUB_PULL_REQUEST_OPENED_ONLY_MESSAGE: &str =
    "Runs only when the pull request opens, not on later pushes.";
pub const GITHUB_PULL_REQUEST_PUSHES_ONLY_MESSAGE: &str =
    "Runs only on pushes after the pull request opens.";
pub const GITHUB_MAIN_PULL_REQUESTS_MESSAGE: &str =
    "Runs only for requests based on main, not stacked requests.";
pub const GITHUB_STACKED_PULL_REQUESTS_MESSAGE: &str =
    "Runs only for stacked requests, not requests based on main.";

const REPRESENTATIVE_REQUEST_ID: &str = "req_00000000000000000000000000000000";

#[derive(Clone, Debug, PartialEq)]
pub enum WorkflowValue {
    Null,
    Bool(bool),
    Integer(i64),
    Real(f64),
    String(String),
    Sequence(Vec<WorkflowValue>),
    Mapping(Vec<(String, WorkflowValue)>),
}

impl WorkflowValue {
    fn get(&self, key: &str) -> Option<&WorkflowValue> {
        self.entries()?
            .iter()
            .find(|(entry_key, _)| entry_key == key)
            .map(|(_, value)| value)
    }

    fn entries(&self) -> Option<&[(String, WorkflowValue)]> {
        match self {
            Self::Mapping(entries) => Some(entries),
            Self::Null
            | Self::Bool(_)
            | Self::Integer(_)
            | Self::Real(_)
            | Self::String(_)
            | Self::Sequence(_) => None,
        }
    }

    fn items(&self) -> Option<&[WorkflowValue]> {
        match self {
            Self::Sequence(items) => Some(items),
            Self::Null
            | Self::Bool(_)
            | Self::Integer(_)
            | Self::Real(_)
            | Self::String(_)
            | Self::Mapping(_) => None,
        }
    }

    fn scalar_text(&self) -> Option<String> {
        match self {
            Self::Bool(value) => Some(value.to_string()),
            Self::Integer(value) => Some(value.to_string()),
            Self::Real(value) => Some(value.to_string()),
            Self::String(value) => Some(value.clone()),
            Self::Null | Self::Sequence(_) | Self::Mapping(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GitHubWorkflowFile {
    pub path: String,
    pub document: Result<WorkflowValue, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowState {
    pub path: String,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GitHubCheckSources {
    pub workflow_files: Vec<GitHubWorkflowFile>,
    pub workflow_states: Vec<GitHubWorkflowState>,
    pub required_on_github: Vec<String>,
    pub reported_on_main: Vec<String>,
    pub opens_pull_requests: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GitHubCheckRuns {
    Yes,
    Maybe(String),
    No(String),
}

impl GitHubCheckRuns {
    fn rank(&self) -> u8 {
        match self {
            Self::Yes => 2,
            Self::Maybe(_) => 1,
            Self::No(_) => 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubWorkflowSummary {
    pub path: String,
    pub name: String,
    pub runs: GitHubCheckRuns,
    pub runtime_named_jobs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCatalogCheck {
    pub name: String,
    pub workflow_path: Option<String>,
    pub runs: Option<GitHubCheckRuns>,
    pub required_on_github: bool,
    pub reported_on_main: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubCheckCatalog {
    pub workflows: Vec<GitHubWorkflowSummary>,
    pub checks: Vec<GitHubCatalogCheck>,
}

pub fn github_check_catalog(sources: GitHubCheckSources) -> GitHubCheckCatalog {
    let pushes = triggers::ScopePushes {
        request_branch: GitHubBranch::Request(REPRESENTATIVE_REQUEST_ID.to_string()).name(),
        base_branch: GitHubBranch::Main.name(),
        opens_pull_requests: sources.opens_pull_requests,
    };
    let mut files = sources.workflow_files.iter().collect::<Vec<_>>();
    files.sort_by(|left, right| left.path.cmp(&right.path));

    let mut workflows = Vec::new();
    let mut checks = BTreeMap::<String, GitHubCatalogCheck>::new();
    let mut budget = jobs::CheckBudget::default();
    for file in files {
        let active = sources
            .workflow_states
            .iter()
            .find(|state| state.path == file.path)
            .is_none_or(|state| state.active);
        let Some((summary, job_checks)) =
            summarize_workflow(file, active, &pushes, &sources.workflow_files, &mut budget)
        else {
            continue;
        };
        for job_check in job_checks {
            let runs = if job_check.conditional && summary.runs == GitHubCheckRuns::Yes {
                GitHubCheckRuns::Maybe(GITHUB_JOB_CONDITION_MESSAGE.to_string())
            } else {
                summary.runs.clone()
            };
            let replaces = checks.get(&job_check.name).is_none_or(|existing| {
                existing
                    .runs
                    .as_ref()
                    .is_none_or(|existing_runs| runs.rank() > existing_runs.rank())
            });
            if replaces {
                checks.insert(
                    job_check.name.clone(),
                    GitHubCatalogCheck {
                        name: job_check.name,
                        workflow_path: Some(file.path.clone()),
                        runs: Some(runs),
                        required_on_github: false,
                        reported_on_main: false,
                    },
                );
            }
        }
        workflows.push(summary);
    }

    for name in &sources.required_on_github {
        listed_check(&mut checks, name).required_on_github = true;
    }
    for name in &sources.reported_on_main {
        listed_check(&mut checks, name).reported_on_main = true;
    }

    GitHubCheckCatalog {
        workflows,
        checks: checks.into_values().collect(),
    }
}

fn listed_check<'a>(
    checks: &'a mut BTreeMap<String, GitHubCatalogCheck>,
    name: &str,
) -> &'a mut GitHubCatalogCheck {
    checks
        .entry(name.to_string())
        .or_insert_with(|| GitHubCatalogCheck {
            name: name.to_string(),
            workflow_path: None,
            runs: None,
            required_on_github: false,
            reported_on_main: false,
        })
}

fn summarize_workflow(
    file: &GitHubWorkflowFile,
    active: bool,
    pushes: &triggers::ScopePushes,
    files: &[GitHubWorkflowFile],
    budget: &mut jobs::CheckBudget,
) -> Option<(GitHubWorkflowSummary, Vec<jobs::JobCheck>)> {
    let file_name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(&file.path)
        .to_string();
    let document = match &file.document {
        Ok(document) => document,
        Err(error) => {
            let summary = GitHubWorkflowSummary {
                path: file.path.clone(),
                name: file_name,
                runs: GitHubCheckRuns::No(format!(
                    "GitHub cannot read this workflow file: {error}"
                )),
                runtime_named_jobs: Vec::new(),
            };
            return Some((summary, Vec::new()));
        }
    };
    let events = triggers::events(document);
    if triggers::only_reusable(&events) {
        return None;
    }
    let runs = if active {
        pushes.verdict(&events)
    } else {
        GitHubCheckRuns::No(GITHUB_WORKFLOW_DISABLED_MESSAGE.to_string())
    };
    let workflow_jobs = jobs::workflow_jobs(document, &file.path, files, budget);
    let summary = GitHubWorkflowSummary {
        path: file.path.clone(),
        name: document
            .get("name")
            .and_then(WorkflowValue::scalar_text)
            .unwrap_or(file_name),
        runs,
        runtime_named_jobs: workflow_jobs.runtime_named,
    };
    Some((summary, workflow_jobs.checks))
}
