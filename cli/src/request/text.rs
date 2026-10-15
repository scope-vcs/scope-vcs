use anyhow::Context;
use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(super) fn discussion_body(
    body: Option<String>,
    body_file: Option<PathBuf>,
) -> anyhow::Result<String> {
    match body_file {
        Some(path) => read_markdown(path),
        None => Ok(body.unwrap_or_default()),
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
    if path == Path::new("-") {
        let mut body = String::new();
        io::stdin()
            .lock()
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
