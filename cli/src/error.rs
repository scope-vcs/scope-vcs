use scope_api_contract::{ErrorCode, ErrorResponse};
use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum ExitCategory {
    Unexpected = 1,
    Usage = 2,
    Authentication = 3,
    Policy = 4,
    StateConflict = 5,
    Temporary = 6,
}

#[derive(Debug)]
pub struct CliError {
    response: ErrorResponse,
    recovery: Option<serde_json::Value>,
}

impl CliError {
    pub fn new(response: ErrorResponse) -> Self {
        Self {
            response,
            recovery: None,
        }
    }

    pub fn partial(message: impl Into<String>, receipt: serde_json::Value) -> Self {
        Self {
            response: ErrorResponse::new(ErrorCode::Conflict, message),
            recovery: Some(receipt),
        }
    }

    pub fn with_recovery(response: ErrorResponse, receipt: serde_json::Value) -> Self {
        Self {
            response,
            recovery: Some(receipt),
        }
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new(ErrorResponse::new(ErrorCode::BadRequest, message))
    }

    pub fn authentication(message: impl Into<String>) -> Self {
        Self::new(ErrorResponse::new(ErrorCode::Unauthorized, message))
    }

    pub fn git_failure(failure: &str, stderr: &str) -> Self {
        let message = if stderr.is_empty() {
            failure.to_string()
        } else {
            format!("{failure}: {stderr}")
        };
        let diagnostic = stderr.to_ascii_lowercase();
        let code = if diagnostic.contains("authentication failed")
            || diagnostic.contains("requested url returned error: 401")
            || diagnostic.contains("could not read username")
        {
            ErrorCode::Unauthorized
        } else if diagnostic.contains("requested url returned error: 403")
            || diagnostic.contains("permission denied")
        {
            ErrorCode::Forbidden
        } else if [
            "could not resolve host:",
            "could not resolve proxy:",
            "failed to connect to ",
            "timed out",
            "timeout was reached",
            "connection timeout",
            "connection refused",
            "connection reset",
            "connection was reset",
            "operation too slow.",
            "empty reply from server",
        ]
        .iter()
        .any(|pattern| {
            diagnostic
                .split(['\r', '\n'])
                .filter(|line| !line.starts_with("remote:"))
                .any(|line| line.contains(pattern))
        }) && !diagnostic.contains("fatal: remote error:")
            && !diagnostic.contains("[remote rejected]")
            && !diagnostic
                .split(['\r', '\n'])
                .any(|line| line.starts_with("remote: error:"))
        {
            ErrorCode::ServiceUnavailable
        } else {
            ErrorCode::Internal
        };
        let response = ErrorResponse::new(code, message);
        Self::new(if code == ErrorCode::ServiceUnavailable {
            response.retryable()
        } else {
            response
        })
    }

    pub fn response(&self) -> &ErrorResponse {
        &self.response
    }

    pub fn exit_category(&self) -> ExitCategory {
        if self.response.retryable {
            return ExitCategory::Temporary;
        }
        match self.response.code {
            ErrorCode::BadRequest | ErrorCode::PayloadTooLarge | ErrorCode::RangeNotSatisfiable => {
                ExitCategory::Usage
            }
            ErrorCode::Unauthorized => ExitCategory::Authentication,
            ErrorCode::CliUpgradeRequired | ErrorCode::Forbidden | ErrorCode::ProtectedPath => {
                ExitCategory::Policy
            }
            ErrorCode::AttachmentUploadExpired
            | ErrorCode::Conflict
            | ErrorCode::NotFound
            | ErrorCode::SharedRepositories => ExitCategory::StateConflict,
            ErrorCode::RequestTimeout
            | ErrorCode::ServiceUnavailable
            | ErrorCode::TooManyRequests => ExitCategory::Temporary,
            ErrorCode::Internal => ExitCategory::Unexpected,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.response.message)?;
        if let Some(instruction) = self.response.instruction.as_deref() {
            write!(formatter, "\n{instruction}")?;
        }
        if let Some(recovery) = &self.recovery {
            if let Some(message) = recovery.get("recovery").and_then(serde_json::Value::as_str) {
                write!(formatter, "\n{message}")?;
            }
            if let Some(commands) = recovery
                .get("recovery_commands")
                .and_then(serde_json::Value::as_array)
            {
                for command in commands {
                    if let Some(command) = command.as_str() {
                        write!(formatter, "\n  {command}")?;
                    } else if let Some(arguments) = command.as_array() {
                        let command = arguments
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(|argument| {
                                if argument
                                    .chars()
                                    .all(|c| c.is_ascii_alphanumeric() || "-_./:=+".contains(c))
                                    && !argument.is_empty()
                                {
                                    argument.to_string()
                                } else {
                                    format!("'{}'", argument.replace('\'', "'\\''"))
                                }
                            })
                            .collect::<Vec<_>>()
                            .join(" ");
                        write!(formatter, "\n  {command}")?;
                    }
                }
            }
        }
        if let Some(error_reference) = self.response.error_reference.as_deref() {
            write!(formatter, "\nReference: {error_reference}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CliError {}

pub fn exit_code(error: &anyhow::Error) -> u8 {
    if let Some(error) = error.downcast_ref::<CliError>() {
        return error.exit_category() as u8;
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_connect() || error.is_timeout())
    {
        return ExitCategory::Temporary as u8;
    }
    ExitCategory::Unexpected as u8
}

pub fn response(error: &anyhow::Error) -> ErrorResponse {
    if let Some(error) = error.downcast_ref::<CliError>() {
        return error.response().clone();
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|error| error.is_connect() || error.is_timeout())
    {
        return ErrorResponse::new(
            ErrorCode::ServiceUnavailable,
            "Scope is temporarily unavailable; retry with bounded backoff",
        )
        .retryable();
    }
    ErrorResponse::new(ErrorCode::Internal, format!("{error:#}"))
}

pub fn json_response(error: &anyhow::Error) -> scope_api_contract::CliFailureEnvelope {
    scope_api_contract::CliFailureEnvelope {
        error: response(error),
        recovery: error
            .downcast_ref::<CliError>()
            .and_then(|error| error.recovery.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;
    use std::{net::TcpListener, time::Duration};

    #[test]
    fn stable_error_codes_map_to_small_process_categories() {
        for (code, expected) in [
            (ErrorCode::BadRequest, ExitCategory::Usage),
            (ErrorCode::Unauthorized, ExitCategory::Authentication),
            (ErrorCode::ProtectedPath, ExitCategory::Policy),
            (ErrorCode::Conflict, ExitCategory::StateConflict),
            (ErrorCode::TooManyRequests, ExitCategory::Temporary),
            (ErrorCode::Internal, ExitCategory::Unexpected),
        ] {
            let error = CliError::new(ErrorResponse::new(code, "fixture"));
            assert_eq!(error.exit_category(), expected);
        }
    }

    #[test]
    fn git_transport_diagnostics_preserve_retry_safety() {
        for diagnostic in [
            "fatal: unable to access 'https://fixture.invalid/': SSL connection timeout",
            "fatal: unable to access 'https://fixture.invalid/': Could not resolve host: fixture.invalid",
            "fatal: unable to access 'https://fixture.invalid/': Resolving timed out after 1000 milliseconds",
            "fatal: unable to access 'https://fixture.invalid/': Timeout was reached",
            "fatal: unable to access 'https://fixture.invalid/': Recv failure: Connection was reset",
            "fatal: unable to access 'https://fixture.invalid/': Recv failure: Connection reset by peer",
            "fatal: unable to access 'https://fixture.invalid/': Operation timed out after 1000 milliseconds",
            "fatal: unable to access 'https://fixture.invalid/': Operation too slow. Less than 1 bytes/sec transferred the last 1 seconds",
            "remote: Enumerating objects: 10, done.\nerror: RPC failed; curl 56 Recv failure: Connection reset by peer",
            "remote: Checking objects...\rerror: RPC failed; curl 56 Recv failure: Connection reset by peer\n",
        ] {
            let error: anyhow::Error = CliError::git_failure("fetch failed", diagnostic).into();
            let response = json_response(&error).error;
            assert_eq!(response.code, ErrorCode::ServiceUnavailable);
            assert!(response.retryable);
        }
        for diagnostic in [
            "remote: connection timeout while applying policy\n! [remote rejected] HEAD -> main (pre-receive hook declined)",
            "fatal: remote error: connection timeout while applying policy",
            "remote: error: connection timeout while applying policy",
            "remote: connection timeout while applying policy",
        ] {
            let error: anyhow::Error = CliError::git_failure("push failed", diagnostic).into();
            assert!(!json_response(&error).error.retryable);
        }
    }

    #[test]
    fn retryable_protocol_skew_is_temporary() {
        let error = CliError::new(ErrorResponse::cli_upgrade_required(Some(
            scope_api_contract::CLI_PROTOCOL_VERSION + 1,
        )));

        assert_eq!(error.exit_category(), ExitCategory::Temporary);
    }

    #[test]
    fn unavailable_api_connections_are_temporary_even_with_context() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let error = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(20))
            .build()
            .unwrap()
            .get(format!("http://{address}/unavailable"))
            .send()
            .context("load fixture")
            .unwrap_err();

        assert_eq!(exit_code(&error), ExitCategory::Temporary as u8);
    }
}
