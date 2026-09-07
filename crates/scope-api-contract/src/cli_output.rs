use serde::{Deserialize, Serialize};

pub const CLI_OUTPUT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(schemars::JsonSchema, ts_rs::TS))]
pub struct CliSuccessEnvelope<T> {
    pub version: u32,
    pub command: String,
    pub result: T,
}

impl<T> CliSuccessEnvelope<T> {
    pub fn new(command: impl Into<String>, result: T) -> Self {
        Self {
            version: CLI_OUTPUT_VERSION,
            command: command.into(),
            result,
        }
    }
}

/// CLI errors preserve the API error shape and can describe completed local/remote effects.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CliFailureEnvelope {
    #[serde(flatten)]
    pub error: crate::ErrorResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<serde_json::Value>,
}
