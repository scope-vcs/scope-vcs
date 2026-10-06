pub fn repository_key(path: &str) -> Option<String> {
    let mut segments = path.strip_prefix("/git/")?.split('/');
    if !is_view_id(segments.next()?) {
        return None;
    }
    let owner = non_empty_segment(segments.next()?)?;
    let repository = non_empty_segment(segments.next()?)?;
    Some(format!("{owner}/{repository}"))
}

fn non_empty_segment(value: &str) -> Option<&str> {
    (!value.is_empty()).then_some(value)
}

fn is_view_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 32
        && bytes[0].is_ascii_lowercase()
        && bytes[1..].iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_' || *byte == b'-'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_the_same_key_from_each_git_operation() {
        for path in [
            "/git/public/scope/router/info/refs",
            "/git/public/scope/router/git-upload-pack",
            "/git/private/scope/router/info/refs",
            "/git/private/scope/router/git-receive-pack",
            "/git/agent_2-docs/scope/router/git-upload-pack",
        ] {
            assert_eq!(repository_key(path).as_deref(), Some("scope/router"));
        }
    }

    #[test]
    fn preserves_canonical_percent_encoded_segments() {
        assert_eq!(
            repository_key("/git/private/an%20owner/a%2Frepo/info/refs").as_deref(),
            Some("an%20owner/a%2Frepo")
        );
    }

    #[test]
    fn rejects_non_git_and_invalid_view_paths() {
        assert_eq!(repository_key("/healthz"), None);
        assert_eq!(repository_key("/git/Agent/scope/router/info/refs"), None);
        assert_eq!(repository_key("/git/1agent/scope/router/info/refs"), None);
        assert_eq!(repository_key("/git/a.b/scope/router/info/refs"), None);
        assert_eq!(
            repository_key(&format!("/git/{}/scope/router/info/refs", "a".repeat(33))),
            None
        );
        assert_eq!(repository_key("/git//scope/router/info/refs"), None);
        assert_eq!(repository_key("/git/public//router/info/refs"), None);
    }
}
