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
        (None, None) => Ok(String::new()),
        (Some(_), Some(_)) => bail!("--body and --body-file cannot be used together"),
    }
}

pub(super) fn append_attachment_references(
    mut markdown: String,
    references: impl IntoIterator<Item = String>,
) -> String {
    let references = references.into_iter().collect::<Vec<_>>().join("\n");
    if references.is_empty() {
        return markdown;
    }
    if markdown.is_empty() {
        return references;
    }
    if markdown.ends_with("\n\n") {
        markdown.push_str(&references);
    } else if markdown.ends_with('\n') {
        markdown.push('\n');
        markdown.push_str(&references);
    } else {
        markdown.push_str("\n\n");
        markdown.push_str(&references);
    }
    markdown
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

#[cfg(test)]
mod tests {
    use super::append_attachment_references;

    #[test]
    fn attachment_references_append_without_placeholder_text() {
        assert_eq!(
            append_attachment_references(
                String::new(),
                ["![shot](/request-attachments/att_one)".into()]
            ),
            "![shot](/request-attachments/att_one)"
        );
        assert_eq!(
            append_attachment_references(
                "Details".into(),
                ["[clip](/request-attachments/att_two)".into()]
            ),
            "Details\n\n[clip](/request-attachments/att_two)"
        );
    }
}
