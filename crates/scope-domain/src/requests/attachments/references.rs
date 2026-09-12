use crate::error::DomainError;
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::collections::BTreeSet;

const REQUEST_ATTACHMENT_REFERENCE_PREFIX: &str = "/request-attachments/";
pub(super) const REQUEST_ATTACHMENT_ID_MAX_BYTES: usize = 128;

/// Extracts attachment IDs from links and images recognized by CommonMark.
pub fn request_attachment_references(markdown: &str) -> Result<BTreeSet<String>, DomainError> {
    let mut references = BTreeSet::new();
    for event in Parser::new_ext(markdown, Options::empty()) {
        let destination = match event {
            Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) => dest_url,
            _ => continue,
        };
        capture_destination(destination.as_ref(), &mut references)?;
    }
    Ok(references)
}

fn capture_destination(
    destination: &str,
    references: &mut BTreeSet<String>,
) -> Result<(), DomainError> {
    let Some(attachment_id) = destination.strip_prefix(REQUEST_ATTACHMENT_REFERENCE_PREFIX) else {
        return Ok(());
    };
    validate_attachment_id(attachment_id)?;
    references.insert(attachment_id.to_string());
    Ok(())
}

pub(super) fn validate_attachment_id(attachment_id: &str) -> Result<(), DomainError> {
    if attachment_id.is_empty()
        || attachment_id.len() > REQUEST_ATTACHMENT_ID_MAX_BYTES
        || !attachment_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(DomainError::invalid_input(
            "request attachment reference has an invalid attachment id",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_inline_and_reference_style_links_and_images() {
        let markdown = r#"
![before](/request-attachments/att_photo)
[recording](</request-attachments/att-video> "title")
[another][proof]

[proof]: /request-attachments/att_reference
"#;
        assert_eq!(
            request_attachment_references(markdown).unwrap(),
            BTreeSet::from([
                "att-video".to_string(),
                "att_photo".to_string(),
                "att_reference".to_string(),
            ])
        );
    }

    #[test]
    fn ignores_plain_text_code_html_and_incomplete_markdown() {
        let markdown = r#"
/request-attachments/plain-text
`![code](/request-attachments/in-code)`

    ![indented](/request-attachments/in-indented-code)

```md
![fenced](/request-attachments/in-fence)
```
<img src="/request-attachments/in-html">
![incomplete](/request-attachments/incomplete
"#;
        assert!(request_attachment_references(markdown).unwrap().is_empty());
    }

    #[test]
    fn supports_multiline_commonmark_destinations() {
        let markdown = "![proof](\n/request-attachments/att_multiline\n)";
        assert_eq!(
            request_attachment_references(markdown).unwrap(),
            BTreeSet::from(["att_multiline".to_string()])
        );
    }

    #[test]
    fn rejects_scope_links_with_queries_or_nested_paths() {
        for destination in [
            "[x](/request-attachments/att?raw=1)",
            "[x](/request-attachments/att/other)",
            "[x](/request-attachments/)",
        ] {
            assert!(request_attachment_references(destination).is_err());
        }
    }
}
