use anyhow::{Context, bail};
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) fn short_oid(oid: &str) -> String {
    oid.chars().take(12).collect()
}

pub(super) fn terminal_text(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

pub(super) fn discussion_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> anyhow::Result<String> {
    let mut stdin = io::stdin().lock();
    discussion_body_with_stdin(body, body_file, &mut stdin)
}

pub(super) fn discussion_body_with_stdin(
    body: Option<String>,
    body_file: Option<PathBuf>,
    stdin: &mut dyn Read,
) -> anyhow::Result<String> {
    match (body, body_file) {
        (Some(body), None) => Ok(body),
        (None, Some(path)) => read_markdown_with_stdin(path, stdin),
        _ => bail!("exactly one of --body or --body-file is required"),
    }
}

pub(super) fn read_markdown(path: PathBuf) -> anyhow::Result<String> {
    read_markdown_with_stdin(path, &mut io::stdin().lock())
}

fn read_markdown_with_stdin(path: PathBuf, stdin: &mut dyn Read) -> anyhow::Result<String> {
    if path == Path::new("-") {
        let mut body = String::new();
        stdin
            .read_to_string(&mut body)
            .context("read Markdown from stdin")?;
        Ok(body)
    } else {
        fs::read_to_string(&path).with_context(|| format!("read Markdown from {}", path.display()))
    }
}
