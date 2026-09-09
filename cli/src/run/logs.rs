use super::{Connection, api, stream};
use serde::Serialize;

#[derive(Serialize)]
struct StoredLog {
    job: String,
    attempt: String,
    step: u32,
    #[serde(flatten)]
    log: scope_api_contract::RepositoryRunLogResponse,
}

#[derive(Serialize)]
struct Logs {
    run_id: String,
    logs: Vec<StoredLog>,
    logs_truncated: bool,
}

pub(super) fn print(
    connection: &Connection,
    run_id: &str,
    job: Option<&str>,
) -> anyhow::Result<()> {
    let detail = connection.detail(run_id)?;
    if let Some(job) = job {
        anyhow::ensure!(
            detail.jobs.iter().any(|j| j.job.key == job),
            crate::error::CliError::usage(format!("job {job} is not part of run {run_id}"))
        );
    }
    let mut result = Logs {
        run_id: run_id.into(),
        logs: Vec::new(),
        logs_truncated: false,
    };
    for item in &detail.jobs {
        if job.is_some_and(|key| key != item.job.key) {
            continue;
        }
        for attempt in &item.attempts {
            for step in &attempt.steps {
                let mut after = 0;
                loop {
                    let page = api::run_step_logs(
                        connection.api(),
                        &connection.target.owner,
                        &connection.target.repo,
                        run_id,
                        &attempt.id,
                        step.index,
                        after,
                    )?;
                    result.logs_truncated |= page.logs_truncated;
                    result.logs.extend(
                        page.logs
                            .into_iter()
                            .filter(|log| log.position > after)
                            .map(|log| StoredLog {
                                job: item.job.key.clone(),
                                attempt: attempt.id.clone(),
                                step: step.index,
                                log,
                            }),
                    );
                    if !page.has_more {
                        break;
                    }
                    anyhow::ensure!(page.next_after > after, "run log cursor did not advance");
                    after = page.next_after;
                }
            }
        }
    }
    result.logs.sort_by_key(|log| log.log.position);
    let mut lines = Vec::new();
    let mut buffers = stream::JobLineBuffers::default();
    for log in &result.logs {
        lines.extend(buffers.push(&log.job, &log.log.text));
    }
    lines.extend(buffers.finish());
    if result.logs_truncated {
        eprintln!("Warning: stored run logs were truncated.");
    }
    crate::execution::emit(
        "run.logs",
        &result,
        lines
            .into_iter()
            .map(|line| line.trim_end_matches('\n').to_string())
            .collect(),
    )
}
