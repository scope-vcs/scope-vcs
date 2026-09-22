use super::super::settings::CloudExecutionSettings;
use anyhow::{Context as _, bail};
use aws_config::BehaviorVersion;
use aws_sdk_lambda::{
    Client as LambdaClient,
    config::{Region, retry::RetryConfig, timeout::TimeoutConfig},
    primitives::Blob,
    types::InvocationType,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone)]
pub(crate) struct EcsClient {
    client: LambdaClient,
    settings: CloudExecutionSettings,
}

#[derive(Debug)]
pub(crate) enum StartError {
    Rejected {
        reason: RejectionReason,
        error: anyhow::Error,
    },
    Ambiguous(anyhow::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RejectionReason {
    Capacity,
    Quota,
    Permanent,
    Authorization,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum StopOutcome {
    Stopped,
    Stopping { stuck: bool },
}

#[derive(Serialize)]
#[serde(tag = "action", rename_all = "snake_case")]
enum BrokerRequest<'a> {
    Start {
        attempt_id: &'a str,
        bootstrap_token: &'a str,
    },
    Stop {
        attempt_id: &'a str,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
enum BrokerReply {
    Started {
        task_arn: String,
    },
    Stopped,
    Stopping {
        stuck: bool,
    },
    Rejected {
        reason: RejectionReason,
        message: String,
    },
    Ambiguous {
        message: String,
    },
}

impl EcsClient {
    pub(crate) async fn new(settings: CloudExecutionSettings) -> Self {
        let config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(settings.aws_region.clone()))
            // The broker has a 120-second Lambda timeout. A lost response remains
            // owned by lease recovery, so this client must not retry dispatch.
            .timeout_config(
                TimeoutConfig::builder()
                    .operation_timeout(Duration::from_secs(150))
                    .operation_attempt_timeout(Duration::from_secs(150))
                    .read_timeout(Duration::from_secs(150))
                    .build(),
            )
            .retry_config(RetryConfig::standard().with_max_attempts(1))
            .load()
            .await;
        Self {
            client: LambdaClient::new(&config),
            settings,
        }
    }

    pub(crate) async fn start(
        &self,
        attempt_id: &str,
        bootstrap_token: &str,
    ) -> Result<String, StartError> {
        match self
            .invoke(&BrokerRequest::Start {
                attempt_id,
                bootstrap_token,
            })
            .await
        {
            Ok(BrokerReply::Started { task_arn }) if !task_arn.trim().is_empty() => Ok(task_arn),
            Ok(BrokerReply::Rejected { reason, message }) => Err(StartError::Rejected {
                reason,
                error: anyhow::anyhow!(message),
            }),
            Ok(BrokerReply::Ambiguous { message }) => {
                Err(StartError::Ambiguous(anyhow::anyhow!(message)))
            }
            Ok(_) => Err(StartError::Ambiguous(anyhow::anyhow!(
                "unexpected dispatch broker start reply"
            ))),
            Err(error) => Err(StartError::Ambiguous(error)),
        }
    }

    pub(crate) async fn stop_terminal_task(&self, attempt_id: &str) -> anyhow::Result<StopOutcome> {
        match self.invoke(&BrokerRequest::Stop { attempt_id }).await? {
            BrokerReply::Stopped => Ok(StopOutcome::Stopped),
            BrokerReply::Stopping { stuck } => Ok(StopOutcome::Stopping { stuck }),
            BrokerReply::Rejected { message, .. } | BrokerReply::Ambiguous { message } => {
                bail!("dispatch broker cleanup incomplete: {message}")
            }
            BrokerReply::Started { .. } => bail!("unexpected dispatch broker stop reply"),
        }
    }

    async fn invoke(&self, request: &BrokerRequest<'_>) -> anyhow::Result<BrokerReply> {
        let response = self
            .client
            .invoke()
            .function_name(&self.settings.dispatch_broker_function_arn)
            .invocation_type(InvocationType::RequestResponse)
            .payload(Blob::new(serde_json::to_vec(request)?))
            .send()
            .await
            .context("invoke dispatch broker")?;
        if response.status_code() != 200 || response.function_error().is_some() {
            bail!("dispatch broker invocation failed");
        }
        let payload = response
            .payload()
            .context("dispatch broker returned no payload")?;
        serde_json::from_slice(payload.as_ref()).context("invalid dispatch broker reply")
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod fake;
