use clap::{CommandFactory, FromArgMatches, Parser, Subcommand, error::ErrorKind};
use scope_cli::api::ApiSession;
use scope_cli::{
    api::{api_url, http_client},
    error::CliError,
    git_credential::run_git_credential,
    git_repo::discover_git_repo,
    login::session_from_cache_or_browser,
    request::{RequestArgs, prepare_request_command, run_request_command},
    run::RunArgs,
    visibility::VisibilityArgs,
};
use std::{path::PathBuf, process::ExitCode};

#[derive(Parser)]
/// Work with Scope repositories, requests, visibility, and cloud runs
#[command(name = "scope")]
#[command(
    after_help = "Start here: scope login, then scope clone owner/repo or scope init --name repo.\nInspect your checkout with scope status. Use --repo owner/repo for remote-only commands."
)]
struct Cli {
    /// Print JSON results; run watch emits JSON lines
    #[arg(long, global = true)]
    json: bool,
    /// Fail instead of opening a browser or prompting for input
    #[arg(long, global = true)]
    non_interactive: bool,
    /// Override the Scope API endpoint
    #[arg(long, global = true, value_name = "URL")]
    api_url: Option<String>,
    /// Select a repository, including outside a checkout
    #[arg(long, global = true, value_name = "OWNER/REPO")]
    repo: Option<String>,
    #[command(subcommand)]
    command: CommandKind,
}

#[derive(Subcommand)]
enum CommandKind {
    /// Create a Scope repository and configure this Git checkout
    Init(InitArgs),
    /// Publish the current commit to Scope main with --main
    Push(PushArgs),
    /// Fetch visible branches and fast-forward the tracked local branch
    Pull(PullArgs),
    /// Edit, inspect, explain, and preview file visibility
    Visibility(VisibilityArgs),
    /// Manage repository contribution rules for coding agents
    Rules(RulesArgs),
    /// Create, inspect, discuss, and merge named requests
    Request(RequestArgs),
    /// Clone a Scope repository and configure Git authentication
    Clone(CloneArgs),
    /// Sign in through a browser, device code, or private exchange file
    Login(LoginArgs),
    /// Revoke the current session and remove its saved credentials
    Logout,
    /// Show the signed-in Scope account
    Whoami,
    /// Explain checkout state, push target, and relevant Scope activity
    Status(InspectionArgs),
    /// Diagnose Git, authentication, endpoint, and local setup problems
    Doctor(InspectionArgs),
    /// Print embedded Scope and third-party licenses, including offline
    Licenses,
    /// Discover workflows, launch runs, inspect logs, and control runs
    Run(RunArgs),
    /// Generate shell completions
    Completions {
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    #[command(name = "git-credential", hide = true)]
    GitCredential(GitCredentialArgs),
}

#[derive(Parser)]
struct InitArgs {
    /// Repository name; required outside an interactive terminal
    #[arg(long)]
    name: Option<String>,
}
#[derive(Parser)]
struct PushArgs {
    /// Explicitly publish HEAD to remote main
    #[arg(long)]
    main: bool,
    /// Scope Git remote to publish to
    #[arg(long)]
    remote: Option<String>,
    /// Skip the TUI and use the local per-worktree visibility config
    #[arg(long)]
    no_review: bool,
    /// Wait for push-triggered workflows to finish
    #[arg(long)]
    wait: bool,
}
#[derive(Parser)]
struct PullArgs {
    /// Scope Git remote to fetch
    #[arg(long)]
    remote: Option<String>,
}
#[derive(Parser)]
struct CloneArgs {
    repository: String,
    destination: Option<PathBuf>,
}
#[derive(Parser)]
struct RulesArgs {
    #[command(subcommand)]
    command: RulesCommand,
}
#[derive(Subcommand)]
enum RulesCommand {
    /// Create rules and synchronize detected agent files
    Sync,
}
#[derive(Parser)]
struct LoginArgs {
    /// Use a device code in a terminal without a browser
    #[arg(long, conflicts_with_all = ["exchange", "exchange_file"])]
    headless: bool,
    #[arg(long, value_name = "TOKEN", conflicts_with = "exchange_file")]
    exchange: Option<String>,
    /// Exchange a token from an owner-only regular file
    #[arg(long, value_name = "PATH", conflicts_with = "exchange")]
    exchange_file: Option<PathBuf>,
}
#[derive(Parser)]
struct GitCredentialArgs {
    operation: String,
}
#[derive(Parser)]
struct InspectionArgs {
    /// Scope Git remote to inspect
    #[arg(long)]
    remote: Option<String>,
    /// Inspect local state without contacting Scope
    #[arg(long)]
    offline: bool,
}

fn main() -> ExitCode {
    let json_requested = std::env::args_os().any(|arg| arg == "--json");
    let matches = match Cli::command()
        .version(scope_cli::build::version_identity())
        .try_get_matches()
    {
        Ok(matches) => matches,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(error) if json_requested => {
            eprintln!(
                "{}",
                serde_json::to_string(&scope_cli::api::ErrorResponse::new(
                    scope_cli::api::ErrorCode::BadRequest,
                    error.to_string()
                ))
                .unwrap()
            );
            return ExitCode::from(2);
        }
        Err(error) => error.exit(),
    };
    let cli = match Cli::from_arg_matches(&matches) {
        Ok(cli) => cli,
        Err(error) => error.exit(),
    };
    let json = cli.json;
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::to_string(&scope_cli::error::json_response(&error)).unwrap()
                );
            } else {
                eprintln!("{error:#}");
            }
            ExitCode::from(scope_cli::error::exit_code(&error))
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    scope_cli::execution::configure(scope_cli::execution::Options {
        json: cli.json,
        non_interactive: cli.non_interactive,
        api_url: cli.api_url,
        repository: cli.repo,
    })?;
    match cli.command {
        CommandKind::Init(args) => scope_cli::init::run(args.name),
        CommandKind::Push(args) => {
            if !args.main {
                return Err(CliError::usage("choose a push destination: use `scope request push` to update a request, or `scope push --main` to publish HEAD to main").into());
            }
            scope_cli::push::run(args.remote.as_deref(), args.no_review, args.wait)
        }
        CommandKind::Pull(args) => scope_cli::pull::run(args.remote.as_deref()),
        CommandKind::Visibility(args) => scope_cli::visibility::run(args),
        CommandKind::Rules(args) => run_rules(args.command),
        CommandKind::Request(args) => run_request(args),
        CommandKind::Clone(args) => {
            scope_cli::clone::clone_repo(&args.repository, args.destination.as_deref())
        }
        CommandKind::Login(args) => {
            scope_cli::login::login(args.headless, args.exchange, args.exchange_file.as_deref())
        }
        CommandKind::Logout => scope_cli::login::logout(),
        CommandKind::Whoami => scope_cli::login::whoami(),
        CommandKind::Status(args) => {
            scope_cli::inspection::status(args.remote.as_deref(), args.offline)
        }
        CommandKind::Doctor(args) => {
            scope_cli::inspection::doctor(args.remote.as_deref(), args.offline)
        }
        CommandKind::Licenses => scope_cli::licenses::run(cli.json),
        CommandKind::Run(args) => scope_cli::run::run_command(args),
        CommandKind::GitCredential(args) => run_git_credential(&args.operation),
        CommandKind::Completions { shell } => {
            if cli.json {
                return Err(
                    CliError::usage("shell completions are shell source; omit --json").into(),
                );
            }
            clap_complete::generate(shell, &mut Cli::command(), "scope", &mut std::io::stdout());
            Ok(())
        }
    }
}
fn run_rules(command: RulesCommand) -> anyhow::Result<()> {
    let repo = discover_git_repo("scope rules")?;
    match command {
        RulesCommand::Sync => {
            let result = scope_cli::agent_context::sync_repo_rules(&repo.root)?;
            let lines = if result.changed_paths.is_empty() {
                vec!["Scope rules context is already in sync.".to_string()]
            } else {
                result
                    .changed_paths
                    .iter()
                    .map(|p| format!("Updated {}", p.display()))
                    .collect()
            };
            scope_cli::execution::emit(
                "rules.sync",
                &serde_json::json!({"changed_paths":result.changed_paths}),
                lines,
            )
        }
    }
}
fn run_request(args: RequestArgs) -> anyhow::Result<()> {
    let command = prepare_request_command(args)?;
    let api_url = api_url();
    let client = http_client()?;
    let session = session_from_cache_or_browser(&client, &api_url)?;
    run_request_command(command, ApiSession::new(&client, &api_url, &session.token))?.render()
}
