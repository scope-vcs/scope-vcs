use super::validation::validate_git_oid;
use super::{
    catalog::RepositoryWorkflowCatalog,
    workflow::{error::WorkflowError, identity::WorkflowPath},
};
use crate::error::DomainError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushWorkflowFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

impl PushWorkflowFile {
    pub fn new(path: impl Into<String>, bytes: Vec<u8>) -> Result<Self, WorkflowError> {
        let path = WorkflowPath::parse(path.into())?;
        Ok(Self {
            path: path.as_str().to_string(),
            bytes,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerInput {
    pub head_oid: String,
    pub workflows: Vec<PushWorkflowFile>,
    pub configuration_error: Option<String>,
}

impl PushTriggerInput {
    pub fn new(
        head_oid: impl Into<String>,
        workflows: Vec<PushWorkflowFile>,
        configuration_error: Option<String>,
    ) -> Result<Self, DomainError> {
        let head_oid = head_oid.into();
        validate_git_oid("push trigger head", &head_oid)?;
        Ok(Self {
            head_oid,
            workflows,
            configuration_error,
        })
    }

    pub fn requires_evaluation(&self) -> bool {
        !self.workflows.is_empty() || self.configuration_error.is_some()
    }
}

impl From<&RepositoryWorkflowCatalog> for PushTriggerInput {
    fn from(catalog: &RepositoryWorkflowCatalog) -> Self {
        let workflows = catalog
            .files()
            .unwrap_or_default()
            .iter()
            .map(|file| PushWorkflowFile {
                path: file.path().as_str().to_string(),
                bytes: file.content_bytes().to_vec(),
            })
            .collect();
        Self {
            head_oid: catalog.source_head_oid().to_string(),
            workflows,
            configuration_error: catalog.configuration_error().map(str::to_string),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PushTriggerEvaluationState {
    Pending,
    Succeeded,
    ConfigurationError,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerCheck {
    pub workflow_path: String,
    pub workflow_name: String,
    pub run_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushTriggerEvaluation {
    pub repository_id: String,
    pub change_version: u64,
    pub head_oid: String,
    pub state: PushTriggerEvaluationState,
    pub message: Option<String>,
    pub checks: Vec<PushTriggerCheck>,
    pub created_at_unix: u64,
    pub completed_at_unix: Option<u64>,
}

impl PushTriggerEvaluation {
    pub fn for_push(
        repository_id: impl Into<String>,
        change_version: u64,
        input: &PushTriggerInput,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let mut evaluation = Self::pending(
            repository_id,
            change_version,
            input.head_oid.clone(),
            now_unix,
        )?;
        if !input.requires_evaluation() {
            evaluation.succeed(Vec::new(), now_unix)?;
        }
        Ok(evaluation)
    }

    pub fn pending(
        repository_id: impl Into<String>,
        change_version: u64,
        head_oid: impl Into<String>,
        now_unix: u64,
    ) -> Result<Self, DomainError> {
        let repository_id = repository_id.into();
        let head_oid = head_oid.into();
        if repository_id.trim().is_empty() || change_version == 0 {
            return Err(DomainError::invalid_input(
                "push trigger evaluation identity is invalid",
            ));
        }
        validate_git_oid("push trigger evaluation head", &head_oid)?;
        Ok(Self {
            repository_id,
            change_version,
            head_oid,
            state: PushTriggerEvaluationState::Pending,
            message: None,
            checks: Vec::new(),
            created_at_unix: now_unix,
            completed_at_unix: None,
        })
    }

    pub fn succeed(
        &mut self,
        checks: Vec<PushTriggerCheck>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::Succeeded,
            None,
            checks,
            now_unix,
        )
    }

    pub fn configuration_error(
        &mut self,
        message: impl Into<String>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::ConfigurationError,
            Some(message.into()),
            Vec::new(),
            now_unix,
        )
    }

    pub fn fail(&mut self, message: impl Into<String>, now_unix: u64) -> Result<(), DomainError> {
        self.finish(
            PushTriggerEvaluationState::Failed,
            Some(message.into()),
            Vec::new(),
            now_unix,
        )
    }

    fn finish(
        &mut self,
        state: PushTriggerEvaluationState,
        message: Option<String>,
        checks: Vec<PushTriggerCheck>,
        now_unix: u64,
    ) -> Result<(), DomainError> {
        if self.state != PushTriggerEvaluationState::Pending {
            return Err(DomainError::conflict(
                "push trigger evaluation is already complete",
            ));
        }
        if now_unix < self.created_at_unix {
            return Err(DomainError::invalid_input(
                "push trigger evaluation completion cannot predate creation",
            ));
        }
        if matches!(
            state,
            PushTriggerEvaluationState::ConfigurationError | PushTriggerEvaluationState::Failed
        ) && message
            .as_deref()
            .is_none_or(|message| message.trim().is_empty())
        {
            return Err(DomainError::invalid_input(
                "failed push trigger evaluation requires a message",
            ));
        }
        self.state = state;
        self.message = message;
        self.checks = checks;
        self.completed_at_unix = Some(now_unix);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runs::catalog::RepositoryWorkflowCatalog;

    const HEAD: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    #[test]
    fn a_push_without_workflows_settles_with_no_checks_and_others_wait_for_evaluation() {
        let without_workflows = PushTriggerInput::new(HEAD, Vec::new(), None).unwrap();
        let settled = PushTriggerEvaluation::for_push("repo-1", 4, &without_workflows, 9).unwrap();
        assert_eq!(settled.state, PushTriggerEvaluationState::Succeeded);
        assert!(settled.checks.is_empty());
        assert_eq!(settled.completed_at_unix, Some(9));

        let workflow = PushWorkflowFile {
            path: "/.scope/runs/checks.yml".to_string(),
            bytes: b"workflow".to_vec(),
        };
        let with_workflow = PushTriggerInput::new(HEAD, vec![workflow], None).unwrap();
        let rejected =
            PushTriggerInput::new(HEAD, Vec::new(), Some("too many workflows".to_string()))
                .unwrap();
        for input in [with_workflow, rejected] {
            let evaluation = PushTriggerEvaluation::for_push("repo-1", 4, &input, 9).unwrap();
            assert_eq!(evaluation.state, PushTriggerEvaluationState::Pending);
            assert_eq!(evaluation.completed_at_unix, None);
        }
    }

    #[test]
    fn push_input_preserves_catalog_capture_errors() {
        let catalog = RepositoryWorkflowCatalog::rejected(
            "repo-1",
            HEAD,
            4,
            "repository contains too many workflows",
        )
        .unwrap();

        let input = PushTriggerInput::from(&catalog);

        assert!(input.workflows.is_empty());
        assert_eq!(
            input.configuration_error.as_deref(),
            Some("repository contains too many workflows")
        );
    }
}
