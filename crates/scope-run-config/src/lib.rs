use scope_domain::runs::{
    cache::definition::{CacheKeyInputs, WorkflowCache},
    catalog::{MAX_WORKFLOW_DEFINITION_BYTES, RepositoryWorkflowCatalog},
    workflow::{
        definition::{
            CompiledWorkflow, ContainerSpec, WorkflowJob, WorkflowJobId, WorkflowStep,
            WorkflowTriggers,
        },
        error::WorkflowError,
        identity::{WorkflowIdentity, WorkflowPath},
        revision::WorkflowRevision,
    },
};
use serde::{Deserialize, Deserializer, de::MapAccess, de::Visitor};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunConfigError {
    #[error("workflow definition exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes")]
    DefinitionTooLarge,
    #[error("workflow definition must be UTF-8: {0}")]
    InvalidUtf8(#[from] std::str::Utf8Error),
    #[error("workflow YAML is invalid: {0}")]
    InvalidYaml(#[from] yaml_serde::Error),
    #[error("workflow push trigger supports only branches: [main]")]
    UnsupportedPushBranches,
    #[error("workflow timeout must be an integer followed by s, m, or h")]
    InvalidTimeout,
    #[error("workflow name {0:?} is defined by more than one file")]
    DuplicateWorkflowName(String),
    #[error("{0}")]
    CatalogRejected(String),
    #[error(transparent)]
    InvalidWorkflow(#[from] WorkflowError),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedWorkflow {
    path: WorkflowPath,
    definition: CompiledWorkflow,
}

impl ParsedWorkflow {
    pub fn path(&self) -> &WorkflowPath {
        &self.path
    }

    pub fn definition(&self) -> &CompiledWorkflow {
        &self.definition
    }

    pub fn into_revision(
        self,
        repository_id: impl Into<String>,
    ) -> Result<WorkflowRevision, WorkflowError> {
        let workflow = WorkflowIdentity::new(repository_id, self.path)?;
        WorkflowRevision::new(workflow, self.definition)
    }
}

pub fn parse_workflow(path: &str, bytes: &[u8]) -> Result<ParsedWorkflow, RunConfigError> {
    if bytes.len() > MAX_WORKFLOW_DEFINITION_BYTES {
        return Err(RunConfigError::DefinitionTooLarge);
    }
    let path = WorkflowPath::parse(path)?;
    let raw: RawWorkflow = yaml_serde::from_str(std::str::from_utf8(bytes)?)?;
    let manual = raw.on.manual;
    let push_main = match raw.on.push {
        None | Some(RawPushTrigger::Enabled(false)) => false,
        Some(RawPushTrigger::Enabled(true)) => true,
        Some(RawPushTrigger::Branches(push)) if push.branches.as_slice() == ["main"] => true,
        Some(RawPushTrigger::Branches(_)) => {
            return Err(RunConfigError::UnsupportedPushBranches);
        }
    };
    let triggers = WorkflowTriggers::new(manual, push_main, raw.on.request)?;
    let container = ContainerSpec::new(raw.container.image)?;
    let timeout_seconds = parse_timeout_seconds(&raw.timeout)?;
    let caches = raw
        .caches
        .into_iter()
        .map(compile_cache)
        .collect::<Result<Vec<_>, _>>()?;
    let environment = raw.environment;
    let jobs = raw
        .jobs
        .0
        .into_iter()
        .map(|(id, job)| {
            let id = WorkflowJobId::parse(id)?;
            let needs = job
                .needs
                .into_iter()
                .map(WorkflowJobId::parse)
                .collect::<Result<Vec<_>, _>>()?;
            let job_container = match job.container {
                Some(container) => ContainerSpec::new(container.image)?,
                None => container.clone(),
            };
            let job_timeout_seconds = match job.timeout {
                Some(timeout) => parse_timeout_seconds(&timeout)?,
                None => timeout_seconds,
            };
            let job_caches = match job.caches {
                Some(caches) => caches
                    .into_iter()
                    .map(compile_cache)
                    .collect::<Result<Vec<_>, _>>()?,
                None => caches.clone(),
            };
            let mut job_environment = environment.clone();
            job_environment.extend(job.environment);
            let steps = job
                .steps
                .into_iter()
                .map(|step| WorkflowStep::new(step.name, step.run))
                .collect::<Result<Vec<_>, _>>()?;
            WorkflowJob::new(
                id,
                needs,
                job_container,
                job_timeout_seconds,
                job_caches,
                job_environment,
                steps,
            )
            .map_err(RunConfigError::from)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let definition = CompiledWorkflow::new(raw.name, triggers, jobs)?;
    Ok(ParsedWorkflow { path, definition })
}

pub fn parse_workflow_set<'a>(
    repository_id: &str,
    workflows: impl IntoIterator<Item = (&'a str, &'a [u8])>,
) -> Result<Vec<WorkflowRevision>, RunConfigError> {
    let mut parsed = workflows
        .into_iter()
        .map(|(path, bytes)| parse_workflow(path, bytes))
        .collect::<Result<Vec<_>, _>>()?;
    parsed.sort_by(|left, right| left.path().cmp(right.path()));

    let mut names = BTreeSet::new();
    let mut revisions = Vec::with_capacity(parsed.len());
    for workflow in parsed {
        let name = workflow.path().name();
        if !names.insert(name.to_string()) {
            return Err(RunConfigError::DuplicateWorkflowName(name.to_string()));
        }
        revisions.push(workflow.into_revision(repository_id)?);
    }
    Ok(revisions)
}

pub fn parse_repository_workflow_catalog(
    catalog: &RepositoryWorkflowCatalog,
) -> Result<Vec<WorkflowRevision>, RunConfigError> {
    if let Some(error) = catalog.configuration_error() {
        return Err(RunConfigError::CatalogRejected(error.to_string()));
    }
    let files = catalog
        .files()
        .expect("a workflow catalog without an error always contains a captured file set")
        .iter()
        .map(|file| (file.path().as_str(), file.content_bytes()));
    parse_workflow_set(catalog.repository_id(), files)
}

fn parse_timeout_seconds(timeout: &str) -> Result<u64, RunConfigError> {
    let timeout = timeout.trim();
    let (number, multiplier) = match timeout.as_bytes().last() {
        Some(b's') => (&timeout[..timeout.len() - 1], 1_u64),
        Some(b'm') => (&timeout[..timeout.len() - 1], 60),
        Some(b'h') => (&timeout[..timeout.len() - 1], 60 * 60),
        _ => return Err(RunConfigError::InvalidTimeout),
    };
    let value = number
        .parse::<u64>()
        .map_err(|_| RunConfigError::InvalidTimeout)?;
    value
        .checked_mul(multiplier)
        .ok_or(RunConfigError::InvalidTimeout)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawWorkflow {
    name: String,
    #[serde(rename = "on")]
    on: RawTriggers,
    container: RawContainer,
    timeout: String,
    #[serde(default)]
    caches: Vec<RawCache>,
    #[serde(default, rename = "env")]
    environment: BTreeMap<String, String>,
    jobs: RawJobs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCache {
    name: String,
    path: String,
    format: String,
    compatibility: RawCacheKeyInputs,
    exact: RawCacheKeyInputs,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCacheKeyInputs {
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    environment: Vec<String>,
    source: bool,
}

fn compile_cache(cache: RawCache) -> Result<WorkflowCache, WorkflowError> {
    WorkflowCache::new(
        cache.name,
        cache.path,
        cache.format,
        CacheKeyInputs::new(
            cache.compatibility.files,
            cache.compatibility.environment,
            cache.compatibility.source,
        )?,
        CacheKeyInputs::new(
            cache.exact.files,
            cache.exact.environment,
            cache.exact.source,
        )?,
    )
    .map_err(WorkflowError::from)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTriggers {
    #[serde(default)]
    manual: bool,
    #[serde(default)]
    push: Option<RawPushTrigger>,
    #[serde(default)]
    request: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RawPushTrigger {
    Enabled(bool),
    Branches(RawPushBranches),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPushBranches {
    branches: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContainer {
    image: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawJob {
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    container: Option<RawContainer>,
    #[serde(default)]
    timeout: Option<String>,
    #[serde(default)]
    caches: Option<Vec<RawCache>>,
    #[serde(default, rename = "env")]
    environment: BTreeMap<String, String>,
    steps: Vec<RawStep>,
}

#[derive(Debug)]
struct RawJobs(Vec<(String, RawJob)>);

impl<'de> Deserialize<'de> for RawJobs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct RawJobsVisitor;

        impl<'de> Visitor<'de> for RawJobsVisitor {
            type Value = RawJobs;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mapping of workflow job IDs to job definitions")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut jobs = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    jobs.push(entry);
                }
                Ok(RawJobs(jobs))
            }
        }

        deserializer.deserialize_map(RawJobsVisitor)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawStep {
    name: String,
    run: String,
}

#[cfg(test)]
mod tests;
