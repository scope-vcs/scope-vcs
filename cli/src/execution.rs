//! Process-wide execution choices, fixed once by the command parser.
use anyhow::Context;
use scope_api_contract::CliSuccessEnvelope;
use serde::Serialize;
use std::{
    io::{self, IsTerminal, Write},
    sync::OnceLock,
};

#[derive(Debug, Default)]
pub struct Options {
    pub json: bool,
    pub non_interactive: bool,
    pub api_url: Option<String>,
    pub repository: Option<String>,
}

static OPTIONS: OnceLock<Options> = OnceLock::new();

pub fn configure(options: Options) -> anyhow::Result<()> {
    if let Some(url) = &options.api_url {
        crate::context::validate_api_url(url)?;
    }
    if let Some(repository) = &options.repository {
        crate::clone::parse_repo_spec(repository)?;
    }
    OPTIONS
        .set(options)
        .map_err(|_| anyhow::anyhow!("CLI execution options were already configured"))
}

pub fn options() -> &'static Options {
    OPTIONS.get_or_init(Options::default)
}

pub fn json() -> bool {
    options().json
}

pub fn interactive() -> bool {
    !options().non_interactive && io::stdin().is_terminal() && io::stderr().is_terminal()
}

pub fn emit<T: Serialize>(
    command: &'static str,
    result: &T,
    human_lines: Vec<String>,
) -> anyhow::Result<()> {
    let mut output = io::stdout().lock();
    if json() {
        serde_json::to_writer(&mut output, &CliSuccessEnvelope::new(command, result))
            .context("serialize command result")?;
        writeln!(output)?;
    } else if !human_lines.is_empty() {
        writeln!(output, "{}", human_lines.join("\n"))?;
    }
    Ok(())
}
