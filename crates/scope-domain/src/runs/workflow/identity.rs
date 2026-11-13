use super::error::WorkflowError;
use crate::runs::validation::is_kebab_name;
use serde::Serialize;

pub const MAX_WORKFLOW_PATH_NAME_BYTES: usize = 64;
const WORKFLOW_PATH_PREFIX: &str = "/.scope/runs/";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct WorkflowPath {
    path: String,
    #[serde(skip)]
    name_end: usize,
}

impl WorkflowPath {
    pub fn parse(path: impl Into<String>) -> Result<Self, WorkflowError> {
        let path = path.into();
        let Some(file_name) = path.strip_prefix(WORKFLOW_PATH_PREFIX) else {
            return Err(WorkflowError::InvalidPath);
        };
        if file_name.contains('/') {
            return Err(WorkflowError::InvalidPath);
        }
        let stem = file_name
            .strip_suffix(".yaml")
            .or_else(|| file_name.strip_suffix(".yml"))
            .ok_or(WorkflowError::InvalidPath)?;
        if !is_kebab_name(stem, MAX_WORKFLOW_PATH_NAME_BYTES) {
            return Err(WorkflowError::InvalidPath);
        }
        let name_end = WORKFLOW_PATH_PREFIX.len() + stem.len();
        Ok(Self { path, name_end })
    }

    pub fn as_str(&self) -> &str {
        &self.path
    }

    pub fn name(&self) -> &str {
        &self.path[WORKFLOW_PATH_PREFIX.len()..self.name_end]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WorkflowIdentity {
    repository_id: String,
    path: WorkflowPath,
}

impl WorkflowIdentity {
    pub fn new(
        repository_id: impl Into<String>,
        path: WorkflowPath,
    ) -> Result<Self, WorkflowError> {
        let repository_id = repository_id.into();
        if repository_id.trim().is_empty() {
            return Err(WorkflowError::MissingRepositoryId);
        }
        Ok(Self {
            repository_id,
            path,
        })
    }

    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    pub fn path(&self) -> &WorkflowPath {
        &self.path
    }
}
