use crate::api::ApiSession;
use crate::{
    api::{self, api_url, http_client_builder, run_detail},
    git_repo::{GitRepo, ensure_git_repo_ready, head_oid, warn_if_dirty_working_tree},
    git_transport::ScopeRemote,
    login::session_from_cache_or_browser,
};
use anyhow::Context;
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use scope_api_contract::{CreateManualRunQuery, ResolveManualRunResponse, RunResponse, RunState};
use std::time::Duration;

mod logs;
mod output;
mod source;
mod stream;

#[derive(Debug, Parser)]
pub struct RunArgs {
    /// Scope remote to use (or select a repository with global --repo).
    #[arg(long, global = true)]
    pub remote: Option<String>,
    #[command(subcommand)]
    pub command: RunCommand,
}

#[derive(Debug, Subcommand)]
pub enum RunCommand {
    /// Start a workflow using the exact local commit; uncommitted edits are excluded.
    Start {
        workflow: String,
        /// Return the queued run without watching it.
        #[arg(long)]
        no_watch: bool,
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
    /// List workflows defined on the repository's current main.
    Workflows,
    /// List recent runs, with an optional workflow filter and continuation cursor.
    List {
        #[arg(long)]
        workflow: Option<String>,
        #[arg(long, default_value_t = 20, value_parser = clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        #[arg(long)]
        after: Option<String>,
    },
    /// Show a run's jobs, attempts, and execution environment.
    Show { run_id: String },
    /// Follow logs and status; reconnect up to five consecutive times. Ctrl-C stops watching only.
    Watch {
        run_id: String,
        /// Maximum seconds to watch, including reconnects.
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
        /// Resume after this log position.
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
    /// Retrieve stored logs; optionally restrict output to one job.
    Logs {
        run_id: String,
        #[arg(long)]
        job: Option<String>,
    },
    /// Request cancellation of a run.
    Cancel { run_id: String },
    /// Retry a run and follow its result.
    Retry {
        run_id: String,
        #[arg(long)]
        no_watch: bool,
        #[arg(long, default_value_t = 1800, value_parser = clap::value_parser!(u64).range(1..))]
        timeout: u64,
    },
}

pub fn run_command(args: RunArgs) -> anyhow::Result<()> {
    let remote = args.remote.as_deref();
    if let RunCommand::Start {
        workflow,
        no_watch,
        timeout,
    } = args.command
    {
        let queued = queue(&workflow, remote)?;
        print_queued("run.start", &queued.run)?;
        return if no_watch {
            Ok(())
        } else {
            queued.connection.watch(&queued.run.id, timeout, 0)
        };
    }
    let connection = Connection::resolve(remote)?;
    match args.command {
        RunCommand::Start { .. } => unreachable!(),
        RunCommand::Workflows => {
            let result = api::run_workflows(
                connection.api(),
                &connection.target.owner,
                &connection.target.repo,
            )?;
            let mut lines: Vec<_> = result
                .workflows
                .iter()
                .map(|w| {
                    format!(
                        "{} · {} · {} jobs · manual: {} · push main: {}",
                        w.key, w.name, w.job_count, w.manual, w.push_main
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("No workflows on current main.".into());
            }
            crate::execution::emit("run.workflows", &result, lines)
        }
        RunCommand::List {
            workflow,
            limit,
            after,
        } => {
            let result = api::run_history(
                connection.api(),
                &connection.target.owner,
                &connection.target.repo,
                workflow.as_deref(),
                limit,
                after.as_deref(),
            )?;
            let mut lines: Vec<_> = result
                .runs
                .iter()
                .map(|r| {
                    format!(
                        "{} · {} · {} · {}",
                        r.id,
                        r.workflow_name,
                        output::run_state_label(r.state),
                        short_oid(&r.git_oid)
                    )
                })
                .collect();
            if lines.is_empty() {
                lines.push("No runs found.".into());
            }
            if let Some(cursor) = &result.next_cursor {
                lines.push(format!("More runs: repeat with --after {cursor}"));
            }
            crate::execution::emit("run.list", &result, lines)
        }
        RunCommand::Show { run_id } => {
            let detail = connection.detail(&run_id)?;
            crate::execution::emit("run.show", &detail, output::detail_lines(&detail))
        }
        RunCommand::Watch {
            run_id,
            timeout,
            after,
        } => connection.watch(&run_id, timeout, after),
        RunCommand::Logs { run_id, job } => logs::print(&connection, &run_id, job.as_deref()),
        RunCommand::Cancel { run_id } => {
            let run = api::cancel_run(
                connection.api(),
                &connection.target.owner,
                &connection.target.repo,
                &run_id,
            )?;
            crate::execution::emit(
                "run.cancel",
                &run,
                vec![format!(
                    "Cancellation requested for {} · {}",
                    run.id,
                    output::run_state_label(run.state)
                )],
            )
        }
        RunCommand::Retry {
            run_id,
            no_watch,
            timeout,
        } => {
            let run = api::retry_run(
                connection.api(),
                &connection.target.owner,
                &connection.target.repo,
                &run_id,
            )?;
            print_queued("run.retry", &run)?;
            if no_watch {
                Ok(())
            } else {
                connection.watch(&run.id, timeout, 0)
            }
        }
    }
}

struct Connection {
    client: Client,
    api_url: String,
    token: String,
    target: ScopeRemote,
}

impl Connection {
    fn api(&self) -> ApiSession<'_> {
        ApiSession::new(&self.client, &self.api_url, &self.token)
    }

    fn resolve(remote: Option<&str>) -> anyhow::Result<Self> {
        let repo = crate::context::discover_optional()?;
        let target = crate::context::resolve_repository(repo.as_ref(), remote)?;
        Self::new(target)
    }

    fn new(target: ScopeRemote) -> anyhow::Result<Self> {
        let api_url = api_url()?;
        let client = run_client(Duration::from_secs(120))?;
        let session = session_from_cache_or_browser(&client, &api_url)?;
        Ok(Self {
            client,
            api_url,
            token: session.token,
            target,
        })
    }

    fn detail(
        &self,
        run_id: &str,
    ) -> anyhow::Result<scope_api_contract::RepositoryRunDetailResponse> {
        run_detail(self.api(), &self.target.owner, &self.target.repo, run_id)
    }

    fn watch(&self, run_id: &str, timeout: u64, after: u64) -> anyhow::Result<()> {
        stream::watch(self, run_id, Duration::from_secs(timeout), after)
    }
}

struct QueuedRun {
    connection: Connection,
    run: RunResponse,
}

fn queue(workflow: &str, remote: Option<&str>) -> anyhow::Result<QueuedRun> {
    let repo = ensure_git_repo_ready("scope run start")?;
    warn_if_dirty_working_tree(&repo)?;
    let target = crate::context::resolve_repository(Some(&repo), remote)?;
    let connection = Connection::new(target)?;
    queue_from_checkout(workflow, &repo, connection)
}

fn queue_from_checkout(
    workflow: &str,
    checkout: &GitRepo,
    connection: Connection,
) -> anyhow::Result<QueuedRun> {
    let query = CreateManualRunQuery {
        workflow: workflow.into(),
        git_oid: head_oid(checkout)?,
        request_id: source::random_request_id()?,
    };
    let run = match api::resolve_manual_run(
        connection.api(),
        &connection.target.owner,
        &connection.target.repo,
        &query,
    )? {
        ResolveManualRunResponse::Queued { run } => {
            eprintln!("Using stored source for {}", short_oid(&query.git_oid));
            run
        }
        ResolveManualRunResponse::UploadRequired => {
            eprintln!("Uploading source for {}", short_oid(&query.git_oid));
            let bundle = source::create_bundle(checkout, &query.request_id, &query.git_oid)?;
            api::create_manual_run(
                connection.api(),
                &connection.target.owner,
                &connection.target.repo,
                &query,
                bundle,
            )?
        }
    };
    Ok(QueuedRun { connection, run })
}

fn print_queued(command: &'static str, run: &RunResponse) -> anyhow::Result<()> {
    crate::execution::emit(
        command,
        run,
        vec![
            format!("Queued {} in Scope Cloud", run.workflow_name),
            format!("Run ID: {}", run.id),
        ],
    )
}

pub fn watch(run_id: &str, remote: Option<&str>) -> anyhow::Result<()> {
    Connection::resolve(remote)?.watch(run_id, 1800, 0)
}

fn run_client(timeout: Duration) -> anyhow::Result<Client> {
    http_client_builder()
        .timeout(timeout)
        .build()
        .context("build run HTTP client")
}

fn is_terminal_state(state: RunState) -> bool {
    scope_domain::runs::run::RunState::from(state).is_terminal()
}
fn short_oid(oid: &str) -> &str {
    oid.get(..7).unwrap_or(oid)
}

/// Wait for a run without writing stdout, for aggregate command receipts.
pub fn wait_completion(run_id: &str, remote: Option<&str>) -> anyhow::Result<RunResponse> {
    stream::completion(
        &Connection::resolve(remote)?,
        run_id,
        Duration::from_secs(1800),
        0,
        false,
    )
}
