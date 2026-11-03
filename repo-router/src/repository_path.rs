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

    proptest::proptest! {
        #![proptest_config(proptest::test_runner::Config { cases: 256, ..Default::default() })]

        #[test]
        fn a_routing_key_is_exactly_the_owner_and_repository_segments(
            segments in proptest::collection::vec("(\\PC|/|\\.\\.|[a-z0-9_-]){0,8}", 0..6)
        ) {
            let path = format!("/git/{}", segments.join("/"));
            match repository_key(&path) {
                Some(key) => {
                    let parts = path.trim_start_matches("/git/").split('/').collect::<Vec<_>>();
                    proptest::prop_assert!(is_view_id(parts[0]));
                    proptest::prop_assert_eq!(key, format!("{}/{}", parts[1], parts[2]));
                    proptest::prop_assert!(!parts[1].is_empty() && !parts[2].is_empty());
                }
                None => {
                    let parts = path.trim_start_matches("/git/").split('/').collect::<Vec<_>>();
                    proptest::prop_assert!(
                        parts.len() < 3 || !is_view_id(parts[0]) || parts[1].is_empty() || parts[2].is_empty()
                    );
                }
            }
        }
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
