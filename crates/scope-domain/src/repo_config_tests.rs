use super::*;

#[test]
fn version_one_config_is_rejected() {
    let mut json = serde_json::to_value(RepoConfig::with_default_view(ViewId::private())).unwrap();
    json["version"] = serde_json::json!(1);
    assert!(matches!(
        RepoConfig::parse_json(&serde_json::to_vec(&json).unwrap()),
        Err(RepoConfigError::InvalidVersion)
    ));
}

#[test]
fn config_default_and_rules_determine_visibility() {
    let config = RepoConfig::parse_json(
        br#"{
            "kind": "scope.repo-config",
            "version": 3,
            "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
            "files": {
                "default": "private",
                "rules": [
                    { "path": "/README.md", "view": "public" },
                    { "path": "/src/**", "view": "public" },
                    { "path": "/src/secrets/**", "view": "private" }
                ]
            }
        }"#,
    )
    .unwrap();

    for (path, expected) in [
        ("/README.md", ViewId::public()),
        ("/src/lib.rs", ViewId::public()),
        ("/src/secrets/key.txt", ViewId::private()),
        ("/notes.txt", ViewId::private()),
    ] {
        assert_eq!(
            config.label_for_path(&ScopePath::parse(path).unwrap()),
            expected
        );
    }
}

#[test]
fn restrictive_rules_match_across_case_and_unicode_while_public_rules_stay_exact() {
    let mut config = RepoConfig::with_default_view(ViewId::public());
    config.files.rules = [
        ("/secrets/**", ViewId::private()),
        ("/Docs/Caf\u{e9}.md", ViewId::private()),
        ("/STRASSE", ViewId::private()),
        (
            "/\u{03c3}\u{03c4}\u{03b9}\u{03b3}\u{03bc}\u{03b1}\u{03c2}",
            ViewId::private(),
        ),
        ("/ma\u{0131}n.txt", ViewId::private()),
        ("/\u{0130}stanbul", ViewId::private()),
        ("/secrets/public.txt", ViewId::public()),
    ]
    .into_iter()
    .map(|(path, view)| RepoConfigFileRule {
        path: path.to_string(),
        view,
    })
    .collect();
    config.history.rewrites = vec![HistoryRewriteRequest {
        path: "/Secrets/**".to_string(),
        action: HistoryRewriteAction::RedactPublicHistory,
    }];
    config.validate().unwrap();

    for (path, expected) in [
        ("/Secrets/key.txt", ViewId::private()),
        ("/SECRETS", ViewId::private()),
        ("/secrets/Nested/KEY", ViewId::private()),
        ("/docs/cafe\u{301}.md", ViewId::private()),
        ("/DOCS/CAF\u{c9}.MD", ViewId::private()),
        ("/stra\u{df}e", ViewId::private()),
        ("/STRA\u{1e9e}E", ViewId::private()),
        (
            "/\u{03c3}\u{03c4}\u{03b9}\u{03b3}\u{03bc}\u{03b1}\u{03c3}",
            ViewId::private(),
        ),
        ("/ma\u{0131}n.txt", ViewId::private()),
        ("/main.txt", ViewId::public()),
        ("/MAIN.TXT", ViewId::public()),
        ("/i\u{0307}stanbul", ViewId::private()),
        ("/istanbul", ViewId::public()),
        ("/secrets/public.txt", ViewId::public()),
        ("/Secrets/public.txt", ViewId::private()),
        ("/secrets2/key.txt", ViewId::public()),
        ("/docs/cafe.md", ViewId::public()),
    ] {
        assert_eq!(
            config.label_for_path(&ScopePath::parse(path).unwrap()),
            expected,
            "{path}"
        );
    }

    let mut private_default = RepoConfig::with_default_view(ViewId::private());
    private_default.files.rules = [
        ("/docs/**", ViewId::public()),
        ("/ma\u{0131}n.txt", ViewId::public()),
        ("/Caf\u{e9}/a.txt", ViewId::public()),
        ("/cafe\u{301}/a.txt", ViewId::private()),
    ]
    .into_iter()
    .map(|(path, view)| RepoConfigFileRule {
        path: path.to_string(),
        view,
    })
    .collect();
    private_default.validate().unwrap();
    for (path, expected) in [
        ("/docs/guide.md", ViewId::public()),
        ("/Docs/guide.md", ViewId::private()),
        ("/ma\u{0131}n.txt", ViewId::public()),
        ("/main.txt", ViewId::private()),
        ("/Caf\u{e9}/a.txt", ViewId::public()),
        ("/cafe\u{301}/a.txt", ViewId::private()),
        ("/CAF\u{c9}/A.TXT", ViewId::private()),
    ] {
        assert_eq!(
            private_default.label_for_path(&ScopePath::parse(path).unwrap()),
            expected,
            "{path}"
        );
    }

    let rewrite = &config.history.rewrites[0];
    assert!(rewrite.matches_path(&ScopePath::parse("/secrets/key.txt").unwrap()));
    assert!(!rewrite.matches_path(&ScopePath::parse("/secrets2/key.txt").unwrap()));
}

#[test]
fn custom_view_cross_spelling_rules_only_narrow_and_fail_closed_on_conflict() {
    use crate::views::{ViewDefinition, ViewIncludes, ViewReaders};
    use std::collections::BTreeSet;

    let agent = ViewId::parse("agent").unwrap();
    let partner = ViewId::parse("partner").unwrap();
    let isolated = ViewId::parse("isolated").unwrap();
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.extend([
        ViewDefinition {
            id: agent.clone(),
            name: "Agent".into(),
            includes: ViewIncludes::Some(BTreeSet::from([ViewId::public()])),
            readers: ViewReaders::Assigned,
        },
        ViewDefinition {
            id: partner.clone(),
            name: "Partner".into(),
            includes: ViewIncludes::Some(BTreeSet::from([ViewId::public()])),
            readers: ViewReaders::Assigned,
        },
        ViewDefinition {
            id: isolated.clone(),
            name: "Isolated".into(),
            includes: ViewIncludes::Some(BTreeSet::new()),
            readers: ViewReaders::Assigned,
        },
    ]);
    let views = Views::new(definitions).unwrap();

    let mut public_default = RepoConfig::with_default_view(ViewId::public());
    public_default.views = views.clone();
    public_default.files.rules = [
        ("/internal/**", agent.clone()),
        ("/isolated/**", isolated.clone()),
        ("/shared/**", agent.clone()),
        ("/SHARED/**", partner.clone()),
        ("/nested/**", agent.clone()),
        ("/NESTED/sub/**", ViewId::private()),
        ("/NESTED/sub/open/**", agent.clone()),
    ]
    .into_iter()
    .map(|(path, view)| RepoConfigFileRule {
        path: path.to_string(),
        view,
    })
    .collect();
    public_default.validate().unwrap();

    for (path, expected) in [
        ("/Internal/a.txt", agent.clone()),
        ("/Isolated/a.txt", ViewId::private()),
        ("/isolated/a.txt", isolated),
        ("/Shared/a.txt", ViewId::private()),
        ("/shared/a.txt", agent.clone()),
        ("/SHARED/a.txt", partner),
        ("/nested/sub/open/a.txt", ViewId::private()),
        ("/NESTED/sub/open/a.txt", agent.clone()),
    ] {
        assert_eq!(
            public_default.label_for_path(&ScopePath::parse(path).unwrap()),
            expected,
            "{path}"
        );
    }

    let mut private_default = RepoConfig::with_default_view(ViewId::private());
    private_default.views = views;
    private_default.files.rules = vec![RepoConfigFileRule {
        path: "/Secret.txt".to_string(),
        view: agent.clone(),
    }];
    private_default.validate().unwrap();
    assert_eq!(
        private_default.label_for_path(&ScopePath::parse("/Secret.txt").unwrap()),
        agent
    );
    assert_eq!(
        private_default.label_for_path(&ScopePath::parse("/secret.txt").unwrap()),
        ViewId::private()
    );
}

#[test]
fn scope_controls_are_private_except_for_canonical_rules() {
    let config = RepoConfig::parse_json(
        br#"{
            "kind": "scope.repo-config",
            "version": 3,
            "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
            "files": {
                "default": "public",
                "rules": []
            }
        }"#,
    )
    .unwrap();

    for path in ["/.scope/repo.json", "/.scope/runs/test.yml", "/.scope"] {
        assert_eq!(
            config.label_for_path(&ScopePath::parse(path).unwrap()),
            ViewId::private()
        );
    }
    assert_eq!(
        config.label_for_path(&ScopePath::parse("/.scope/RULES.md").unwrap()),
        ViewId::public()
    );

    let mut private_config = RepoConfig::with_default_view(ViewId::private());
    private_config.files.rules.push(RepoConfigFileRule {
        path: "/.scope/RULES.md".to_string(),
        view: ViewId::private(),
    });
    assert_eq!(
        private_config.label_for_path(&ScopePath::parse("/.scope/RULES.md").unwrap()),
        ViewId::public()
    );
}

#[test]
fn scope_control_rules_and_rewrites_are_rejected() {
    for (path, visibility) in [
        ("/.scope/**", "public"),
        ("/.scope/runs/test.yml", "private"),
        ("/.scope/RULES.md", "private"),
    ] {
        let views_json = serde_json::to_string(&Views::builtin()).unwrap();
        let json = format!(
            r#"{{
                "kind": "scope.repo-config",
                "version": 3,
                "views":{views_json},
                "files": {{
                    "default": "private",
                    "rules": [{{ "path": "{path}", "view": "{visibility}" }}]
                }}
            }}"#
        );
        let error = RepoConfig::parse_json(json.as_bytes()).unwrap_err();
        assert!(matches!(error, RepoConfigError::ReservedControlPath(_)));
    }
    let error = RepoConfig::parse_json(
        br#"{
            "kind":"scope.repo-config","version":3,"views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
            "files":{"default":"private","rules":[]},
            "history":{"rewrites":[{"path":"/.scope/RULES.md","action":"redact-public-history"}]}
        }"#,
    )
    .unwrap_err();
    assert!(matches!(error, RepoConfigError::ReservedControlPath(_)));
}

#[test]
fn non_canonical_paths_are_rejected_in_rules_and_rewrites() {
    for path in ["/secrets//**", "/secrets/** ", "/README.md "] {
        for section in [
            format!(r#""rules":[{{"path":"{path}","view":"private"}}]"#),
            format!(
                r#""rules":[]}},"history":{{"rewrites":[{{"path":"{path}","action":"redact-public-history"}}]"#
            ),
        ] {
            let views_json = serde_json::to_string(&Views::builtin()).unwrap();
            let json = format!(
                r#"{{"kind":"scope.repo-config","version":3,"views":{views_json},"files":{{"default":"public",{section}}}}}"#
            );
            let error = RepoConfig::parse_json(json.as_bytes()).unwrap_err();
            assert!(
                matches!(
                    error,
                    RepoConfigError::InvalidSegment | RepoConfigError::RelativePath
                ),
                "{path} should be rejected, got {error:?}"
            );
        }
    }
}

#[test]
fn history_rewrites_are_accepted_for_supported_actions() {
    let config = RepoConfig::parse_json(
        br#"{
            "kind": "scope.repo-config",
            "version": 3,
            "views":[{"id":"public","name":"Public","includes":[],"readers":"anyone"},{"id":"private","name":"Private","includes":"all","readers":"assigned"}],
            "files": {
                "default": "private",
                "rules": []
            },
            "history": {
                "rewrites": [
                    {
                        "path": "/secret.md",
                        "action": "redact-public-history"
                    }
                ]
            }
        }"#,
    )
    .unwrap();

    let rewrite = &config.history.rewrites[0];
    assert_eq!(rewrite.action, HistoryRewriteAction::RedactPublicHistory);
    assert!(rewrite.matches_path(&ScopePath::parse("/secret.md").unwrap()));
    assert!(!rewrite.matches_path(&ScopePath::parse("/public.md").unwrap()));
}
