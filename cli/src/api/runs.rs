use super::*;
use crate::api::ApiSession;
use anyhow::Context;
use scope_api_contract::{
    CreateManualRunQuery, PushTriggerEvaluationResponse, RepositoryRunDetailResponse,
    ResolveManualRunResponse, RunEventsQuery, RunLogResponse, RunResponse,
};
use std::io::BufRead;

pub fn get_push_trigger_evaluation(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    head_oid: &str,
) -> anyhow::Result<PushTriggerEvaluationResponse> {
    decode_json_response(
        api.request(
            reqwest::Method::GET,
            routes::repo_push_trigger_evaluation(owner, repo, head_oid),
        )
        .send()
        .context("load Scope push trigger evaluation")?,
        "load Scope push trigger evaluation",
    )
}

pub fn resolve_manual_run(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    query: &CreateManualRunQuery,
) -> anyhow::Result<ResolveManualRunResponse> {
    decode_json_response(
        api.request(reqwest::Method::POST, routes::repo_run_resolve(owner, repo))
            .query(query)
            .send()
            .context("resolve Scope run source")?,
        "resolve Scope run source",
    )
}

pub fn create_manual_run(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    query: &CreateManualRunQuery,
    bundle: Vec<u8>,
) -> anyhow::Result<RunResponse> {
    decode_json_response(
        api.request(reqwest::Method::POST, routes::repo_runs(owner, repo))
            .query(query)
            .header("content-type", "application/octet-stream")
            .body(bundle)
            .send()
            .context("create Scope run")?,
        "create Scope run",
    )
}

pub enum RunStreamEvent {
    Log(RunLogResponse),
    Status(RunResponse),
}

pub fn run_detail(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RepositoryRunDetailResponse> {
    decode_json_response(
        api.request(
            reqwest::Method::GET,
            routes::repo_run_detail(owner, repo, run_id),
        )
        .send()
        .context("load Scope run detail")?,
        "load Scope run detail",
    )
}

pub fn stream_run_events(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    run_id: &str,
    after: u64,
    on_event: impl FnMut(RunStreamEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    let response = successful_response(
        api.request(
            reqwest::Method::GET,
            routes::repo_run_events(owner, repo, run_id),
        )
        .query(&RunEventsQuery { after })
        .send()
        .context("watch Scope run")?,
        "watch Scope run",
    )?;
    parse_run_event_stream(std::io::BufReader::new(response), on_event)
}

fn parse_run_event_stream(
    reader: impl BufRead,
    mut on_event: impl FnMut(RunStreamEvent) -> anyhow::Result<bool>,
) -> anyhow::Result<()> {
    let mut event_name = String::new();
    let mut data = Vec::new();
    for line in reader.lines() {
        let line = line.context("read Scope run event stream")?;
        if line.is_empty() {
            if !data.is_empty() {
                let payload = data.join("\n");
                let event = match event_name.as_str() {
                    "log" => Some(RunStreamEvent::Log(
                        serde_json::from_str(&payload).context("parse Scope run log event")?,
                    )),
                    "status" => Some(RunStreamEvent::Status(
                        serde_json::from_str(&payload).context("parse Scope run status event")?,
                    )),
                    "error" => {
                        let error: ErrorResponse = serde_json::from_str(&payload)
                            .context("parse Scope run stream error")?;
                        return Err(crate::error::CliError::new(terminal_safe_error_response(
                            error,
                        ))
                        .into());
                    }
                    _ => None,
                };
                if let Some(event) = event
                    && !on_event(event)?
                {
                    return Ok(());
                }
            }
            event_name.clear();
            data.clear();
            continue;
        }
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = value.trim_start().to_string();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push(value.trim_start().to_string());
        }
    }
    Ok(())
}

pub fn cancel_run(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RunResponse> {
    mutate_run(
        api,
        routes::repo_run_cancel(owner, repo, run_id),
        "cancel Scope run",
    )
}

pub fn retry_run(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> anyhow::Result<RunResponse> {
    mutate_run(
        api,
        routes::repo_run_retry(owner, repo, run_id),
        "retry Scope run",
    )
}

fn mutate_run(api: ApiSession<'_>, path: String, context: &str) -> anyhow::Result<RunResponse> {
    decode_json_response(
        api.request(reqwest::Method::POST, path)
            .send()
            .with_context(|| context.to_string())?,
        context,
    )
}

pub fn run_workflows(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
) -> anyhow::Result<RepositoryRunWorkflowListResponse> {
    decode_json_response(
        api.request(
            reqwest::Method::GET,
            routes::repo_run_workflows(owner, repo),
        )
        .send()
        .context("list run workflows")?,
        "list run workflows",
    )
}

pub fn run_history(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    workflow: Option<&str>,
    limit: u32,
    after: Option<&str>,
) -> anyhow::Result<RepositoryRunHistoryPageResponse> {
    let mut query = vec![("limit", limit.to_string())];
    if let Some(workflow) = workflow {
        query.push(("workflow", workflow.into()));
    }
    if let Some(after) = after {
        query.push(("after", after.into()));
    }
    decode_json_response(
        api.request(reqwest::Method::GET, routes::repo_runs(owner, repo))
            .query(&query)
            .send()
            .context("list runs")?,
        "list runs",
    )
}

pub fn run_step_logs(
    api: ApiSession<'_>,
    owner: &str,
    repo: &str,
    run_id: &str,
    attempt: &str,
    step: u32,
    after: u64,
) -> anyhow::Result<RepositoryRunStepLogPageResponse> {
    decode_json_response(
        api.request(
            reqwest::Method::GET,
            routes::repo_run_step_logs(owner, repo, run_id, attempt, step),
        )
        .query(&[("after", after)])
        .send()
        .context("load run logs")?,
        "load run logs",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{CliError, ExitCategory};
    use std::io::Cursor;

    #[test]
    fn run_stream_errors_preserve_the_safe_diagnostic_reference() {
        let stream = concat!(
            "event: error\n",
            "data: {\"code\":\"internal\",\"message\":\"Scope hit an internal error.\",",
            "\"error_reference\":\"err_0123456789abcdef0123456789abcdef\",",
            "\"retryable\":false}\n\n",
        );

        let error = parse_run_event_stream(Cursor::new(stream), |_| Ok(true)).unwrap_err();
        let error = error.downcast_ref::<CliError>().expect("typed CLI error");

        assert_eq!(error.exit_category(), ExitCategory::Unexpected);
        assert_eq!(
            error.to_string(),
            "Scope hit an internal error.\nReference: err_0123456789abcdef0123456789abcdef"
        );
    }
}
