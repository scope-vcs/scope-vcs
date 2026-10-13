use super::RuntimeClient;
use scope_api_contract::{AttemptConclusionRequest, AttemptStatusResponse, CompleteAttemptRequest};
use scope_domain::runs::{exit_code::SetupFailure, step::SetupFailureMessage};

impl RuntimeClient {
    pub fn complete_succeeded(&self, logs_truncated: bool) -> anyhow::Result<()> {
        self.complete(AttemptConclusionRequest::Succeeded, logs_truncated)
    }

    pub fn complete_setup_failure(&self, message: &str) -> anyhow::Result<()> {
        let message = SetupFailureMessage::normalized(message).into_string();
        self.complete(
            AttemptConclusionRequest::SetupFailed {
                exit_code: SetupFailure::RuntimeSetup.exit_code(),
                message,
            },
            false,
        )
    }

    pub(super) fn complete(
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
}
