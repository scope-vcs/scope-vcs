use super::{RuntimeClient, ensure_success};
use anyhow::Context as _;
use scope_api_contract::{
    AttemptConclusionRequest, AttemptStatusResponse, CompleteAttemptRequest,
    CompleteAttemptStepRequest, StepConclusionRequest,
};
use scope_domain::runs::{exit_code::SetupFailure, step::StepConclusion};

impl RuntimeClient {
    pub fn complete_step(
        &self,
        step: u32,
        exit_code: i32,
        logs_truncated: bool,
    ) -> anyhow::Result<AttemptStatusResponse> {
        let conclusion = match StepConclusion::from_exit_code(exit_code) {
            StepConclusion::Succeeded => StepConclusionRequest::Succeeded,
            StepConclusion::Failed { exit_code } => StepConclusionRequest::Failed { exit_code },
        };
        self.post_json(
            &format!("steps/{step}/complete"),
            &CompleteAttemptStepRequest {
                conclusion,
                logs_truncated,
            },
            "complete step",
        )
    }

    pub fn complete_timeout(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::TimedOut, logs_truncated)
    }

    pub fn complete_succeeded(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Succeeded, logs_truncated)
    }

    pub fn complete_canceled(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Canceled, logs_truncated)
    }

    pub fn complete_setup_failure(&self, message: &str) -> anyhow::Result<()> {
        self.complete(
            AttemptConclusionRequest::SetupFailed {
                exit_code: SetupFailure::RuntimeSetup.exit_code(),
                message: message.chars().take(2048).collect(),
            },
            false,
        )
    }

    fn complete(
        &self,
        conclusion: AttemptConclusionRequest,
        logs_truncated: bool,
    ) -> anyhow::Result<()> {
        let _: AttemptStatusResponse = self.post_json(
            "complete",
            &CompleteAttemptRequest {
                conclusion,
                logs_truncated,
            },
            "complete attempt",
        )?;
        Ok(())
    }

    pub fn abandon(&self) -> anyhow::Result<()> {
        let response = self
            .auth(self.client.post(self.url("abandon")))
            .send()
            .context("abandon attempt")?;
        ensure_success(&response, "abandon attempt")
    }
}
