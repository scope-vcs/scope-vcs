use super::{Connection, api, is_terminal_state, output, run_client, short_oid, state_label};
use crate::api::ApiSession;
use crate::api::RunStreamEvent;
use scope_api_contract::{RunResponse, RunState};
use std::{
    thread,
    time::{Duration, Instant},
};

const MAX_PARTIAL_JOB_LINE_BYTES: usize = 8 * 1_024;
const MAX_RECONNECTS: u32 = 5;

pub(super) fn watch(
    connection: &Connection,
    run_id: &str,
    timeout: Duration,
    after: u64,
) -> anyhow::Result<()> {
    completion(connection, run_id, timeout, after, true).map(|_| ())
}

pub(super) fn completion(
    connection: &Connection,
    run_id: &str,
    timeout: Duration,
    after: u64,
    emit: bool,
) -> anyhow::Result<RunResponse> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| crate::error::CliError::usage("watch timeout is too large"))?;
    let mut cursor = after;
    let mut reconnects = 0;
    let mut line_buffers = JobLineBuffers::default();
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            print_job_lines(line_buffers.finish());
            return Err(temporary(format!(
                "watch timed out; resume with {}",
                resume_command(connection, run_id, cursor)
            )));
        }
        // A quiet SSE connection can remain healthy through server keep-alives.
        // Only the overall watch deadline should time it out.
        let client = run_client(remaining)?;
        let api = ApiSession::new(&client, &connection.api_url, &connection.token);
        let mut terminal = None;
        let previous_cursor = cursor;
        let result = api::stream_run_events(
            api,
            &connection.target.owner,
            &connection.target.repo,
            run_id,
            cursor,
            |event| {
                match event {
                    RunStreamEvent::Log(log) if advance_log_cursor(&mut cursor, log.position) => {
                        if emit {
                            if crate::execution::json() {
                                crate::execution::emit("run.log", &log, Vec::new())?;
                            } else {
                                print_job_lines(line_buffers.push(&log.job_key, &log.text));
                            }
                        }
                    }
                    RunStreamEvent::Status(run) => {
                        if emit && crate::execution::json() {
                            crate::execution::emit("run.status", &run, Vec::new())?;
                        }
                        if is_terminal_state(run.state) {
                            terminal = Some(run);
                        }
                    }
                    _ => {}
                }
                Ok(terminal.is_none())
            },
        );
        if let Some(run) = terminal {
            if emit && !crate::execution::json() {
                print_job_lines(line_buffers.finish());
                println!(
                    "Run {} · {}",
                    state_label(run.state),
                    short_oid(&run.git_oid)
                );
                let summary = run_client(Duration::from_secs(3)).and_then(|client| {
                    api::run_detail(
                        ApiSession::new(&client, &connection.api_url, &connection.token),
                        &connection.target.owner,
                        &connection.target.repo,
                        run_id,
                    )
                });
                if let Ok(detail) = summary {
                    for line in output::detail_lines(&detail).into_iter().skip(1) {
                        println!("{line}");
                    }
                }
            }
            if run.logs_truncated {
                eprintln!("Warning: stored run logs were truncated.");
            }
            return if run.state == RunState::Succeeded {
                Ok(run)
            } else {
                Err(
                    crate::error::CliError::new(scope_api_contract::ErrorResponse::new(
                        scope_api_contract::ErrorCode::Conflict,
                        format!("run {} {}", run.id, state_label(run.state)),
                    ))
                    .into(),
                )
            };
        }
        if let Err(error) = result {
            let retryable = error.downcast_ref::<reqwest::Error>().is_some()
                || error.downcast_ref::<std::io::Error>().is_some()
                || error
                    .downcast_ref::<crate::error::CliError>()
                    .is_some_and(|e| e.exit_category() == crate::error::ExitCategory::Temporary);
            if !retryable {
                print_job_lines(line_buffers.finish());
                return Err(error);
            }
            eprintln!("Run stream interrupted: {error}; reconnecting after log {cursor}.");
        }
        if cursor > previous_cursor {
            reconnects = 0;
        }
        reconnects += 1;
        if reconnects > MAX_RECONNECTS {
            if emit && !crate::execution::json() {
                print_job_lines(line_buffers.finish());
            }
            return Err(temporary(format!(
                "run stream disconnected {MAX_RECONNECTS} times; resume with {}",
                resume_command(connection, run_id, cursor)
            )));
        }
        thread::sleep(
            Duration::from_secs(1 << (reconnects - 1))
                .min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn resume_command(connection: &Connection, run_id: &str, cursor: u64) -> String {
    format!(
        "scope --repo {}/{} run watch {run_id} --after {cursor}",
        connection.target.owner, connection.target.repo
    )
}

fn temporary(message: String) -> anyhow::Error {
    crate::error::CliError::new(
        scope_api_contract::ErrorResponse::new(
            scope_api_contract::ErrorCode::ServiceUnavailable,
            message,
        )
        .retryable(),
    )
    .into()
}

fn advance_log_cursor(cursor: &mut u64, position: u64) -> bool {
    if position <= *cursor {
        return false;
    }
    *cursor = position;
    true
}

#[derive(Default)]
pub(super) struct JobLineBuffers {
    active_job: Option<String>,
    partial: String,
}

impl JobLineBuffers {
    pub(super) fn push(&mut self, job: &str, text: &str) -> Vec<String> {
        let mut lines = Vec::new();
        if self
            .active_job
            .as_deref()
            .is_some_and(|active| active != job)
        {
            lines.extend(self.finish());
        }
        self.active_job.get_or_insert_with(|| job.to_string());
        self.partial.push_str(text);
        while let Some(end) = job_line_end(&self.partial) {
            let line = self.partial.drain(..end).collect::<String>();
            lines.push(format!("[{job}] {}\n", line.trim_end_matches(['\r', '\n'])));
        }
        while self.partial.len() > MAX_PARTIAL_JOB_LINE_BYTES {
            let end = bounded_char_end(&self.partial, MAX_PARTIAL_JOB_LINE_BYTES);
            let line = self.partial.drain(..end).collect::<String>();
            lines.push(format!("[{job}] {line}\n"));
        }
        if self.partial.is_empty() {
            self.active_job = None;
        }
        lines
    }

    pub(super) fn finish(&mut self) -> Vec<String> {
        let Some(job) = self.active_job.take() else {
            return Vec::new();
        };
        let line = std::mem::take(&mut self.partial);
        let line = line.trim_end_matches(['\r', '\n']);
        if line.is_empty() {
            Vec::new()
        } else {
            vec![format!("[{job}] {line}\n")]
        }
    }
}

fn job_line_end(text: &str) -> Option<usize> {
    let newline = text.find('\n').map(|index| index + 1);
    let carriage_return = text.find('\r').and_then(|index| {
        let following = text.as_bytes().get(index + 1)?;
        Some(index + if *following == b'\n' { 2 } else { 1 })
    });
    match (newline, carriage_return) {
        (Some(newline), Some(carriage_return)) => Some(newline.min(carriage_return)),
        (Some(newline), None) => Some(newline),
        (None, Some(carriage_return)) => Some(carriage_return),
        (None, None) => None,
    }
}

fn bounded_char_end(text: &str, limit: usize) -> usize {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn print_job_lines(lines: Vec<String>) {
    for line in lines {
        print!("{line}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_watch_ignores_replayed_log_positions() {
        let mut cursor = 7;
        assert!(!advance_log_cursor(&mut cursor, 6));
        assert!(!advance_log_cursor(&mut cursor, 7));
        assert!(advance_log_cursor(&mut cursor, 8));
        assert_eq!(cursor, 8);
    }

    #[test]
    fn run_watch_preserves_job_order_when_partial_lines_interleave() {
        let mut buffers = JobLineBuffers::default();
        assert!(buffers.push("backend", "compiling").is_empty());
        assert_eq!(
            buffers.push("web", "testing\n"),
            ["[backend] compiling\n", "[web] testing\n"],
        );
        assert_eq!(
            buffers.push("backend", " complete\nnext"),
            ["[backend]  complete\n"],
        );
        assert_eq!(buffers.finish(), ["[backend] next\n"]);
    }

    #[test]
    fn run_watch_flushes_carriage_returns_and_bounds_partial_lines() {
        let mut buffers = JobLineBuffers::default();
        assert_eq!(
            buffers.push("web", "building 10%\rbuilding 20%\r"),
            ["[web] building 10%\n"],
        );
        assert_eq!(
            buffers.push("web", "done\n"),
            ["[web] building 20%\n", "[web] done\n",]
        );

        let long_line = "x".repeat(MAX_PARTIAL_JOB_LINE_BYTES + 1);
        let flushed = buffers.push("web", &long_line);
        assert_eq!(flushed.len(), 1);
        assert_eq!(
            flushed[0].len(),
            MAX_PARTIAL_JOB_LINE_BYTES + "[web] \n".len()
        );
        assert_eq!(buffers.finish(), ["[web] x\n"]);
    }

    #[test]
    fn run_watch_preserves_crlf_split_across_chunks() {
        let mut buffers = JobLineBuffers::default();
        assert!(buffers.push("windows", "complete\r").is_empty());
        assert_eq!(buffers.push("windows", "\n"), ["[windows] complete\n"]);
    }
}
