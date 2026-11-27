use super::*;

fn config(default: ViewId, rules: Vec<(&str, ViewId)>) -> RepoConfig {
    let mut config = RepoConfig::with_default_view(default);
    config.files.rules = rules
        .into_iter()
        .map(|(path, visibility)| RepoConfigFileRule {
            path: path.to_string(),
            view: visibility,
        })
        .collect();
    config.validate().unwrap();
    config
}

fn directory<'a>(path: &'a str, file_paths_under: Vec<&'a str>) -> VisibilityTarget<'a> {
    VisibilityTarget {
        name: path.rsplit('/').next().unwrap_or(path),
        path,
        kind: VisibilityNodeKind::Directory,
        reserved: false,
        file_paths_under,
    }
}

fn file(path: &str) -> VisibilityTarget<'_> {
    VisibilityTarget {
        name: path.rsplit('/').next().unwrap_or(path),
        path,
        kind: VisibilityNodeKind::File,
        reserved: false,
        file_paths_under: vec![path],
    }
}

#[test]
fn folder_toggle_canonicalizes_subtree_rules() {
    let public = ViewId::public();
    let private = ViewId::private();
    let cases = [
        (
            vec![("/src/private.clone()/**", public.clone())],
            vec!["/src/lib.rs", "/src/private.clone()/key"],
            vec![("/src/**", public.clone())],
        ),
        (
            vec![("/src/lib.rs", public.clone())],
            vec!["/src/lib.rs", "/src/secret.rs"],
            vec![("/src/**", public.clone())],
        ),
        (
            vec![("/src/a.rs", public.clone()), ("/src/b.rs", public.clone())],
            vec!["/src/a.rs", "/src/b.rs"],
            vec![],
        ),
        (
            vec![
                ("/src/lib.rs", public.clone()),
                ("/src/secrets/**", private.clone()),
            ],
            vec!["/src/lib.rs", "/src/secrets/key"],
            vec![
                ("/src/**", public.clone()),
                ("/src/secrets/**", private.clone()),
            ],
        ),
        (
            vec![("/src/**", public), ("/src/secrets/**", private.clone())],
            vec!["/src/lib.rs", "/src/secrets/key"],
            vec![],
        ),
    ];
    for (rules, paths, expected) in cases {
        let mut config = config(private.clone(), rules);
        assert!(toggle_visibility_target(&mut config, directory("/src", paths)).changed);
        assert_eq!(
            config.files.rules,
            expected
                .into_iter()
                .map(|(path, visibility)| RepoConfigFileRule {
                    path: path.to_string(),
                    view: visibility,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn file_toggle_removes_stale_same_base_folder_rule() {
    let mut config = config(ViewId::private(), vec![("/docs/**", ViewId::public())]);
    let target = file("/docs");

    assert_eq!(
        target_visibility(&config, &target),
        ReviewLabel::View(ViewId::public())
    );
    toggle_visibility_target(&mut config, target.clone());

    assert!(config.files.rules.is_empty());
    assert_eq!(
        target_visibility(&config, &target),
        ReviewLabel::View(ViewId::private())
    );
}

#[test]
fn file_toggle_refuses_paths_that_collide_with_subtree_pattern_syntax() {
    let mut config = config(ViewId::private(), vec![]);

    let result = toggle_visibility_target(&mut config, file("/src/**"));

    assert!(!result.changed);
    assert!(result.message.contains("pattern syntax"));
    assert!(config.files.rules.is_empty());
}

#[test]
fn canonicalization_preserves_same_base_exact_and_subtree_semantics() {
    let public = ViewId::public();
    let private = ViewId::private();
    let cases = [
        (
            private.clone(),
            vec![("/docs/**", public.clone()), ("/docs", private.clone())],
            vec![("/docs/**", public.clone()), ("/docs", private.clone())],
        ),
        (
            public.clone(),
            vec![("/docs/**", private.clone()), ("/docs", private.clone())],
            vec![("/docs/**", private.clone())],
        ),
        (
            public.clone(),
            vec![("/docs", public.clone()), ("/docs/**", private.clone())],
            vec![("/docs", public), ("/docs/**", private.clone())],
        ),
    ];
    for (default, rules, expected) in cases {
        let mut config = config(default, rules);
        canonicalize_visibility_rules(&mut config);
        assert_eq!(
            effective_config_label_for_path(&config, "/docs"),
            private.clone()
        );
        assert_eq!(
            config.files.rules,
            expected
                .into_iter()
                .map(|(path, visibility)| RepoConfigFileRule {
                    path: path.to_string(),
                    view: visibility,
                })
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn reserved_scope_paths_cannot_be_toggled_public() {
    let mut config = config(ViewId::public(), vec![]);
    let target = VisibilityTarget {
        name: "test.yml",
        path: "/.scope/runs/test.yml",
        kind: VisibilityNodeKind::File,
        reserved: true,
        file_paths_under: vec!["/.scope/runs/test.yml"],
    };

    let result = toggle_visibility_target(&mut config, target.clone());

    assert!(!result.changed);
    assert!(config.files.rules.is_empty());
    assert_eq!(
        target_visibility(&config, &target),
        ReviewLabel::View(ViewId::private())
    );
}

#[test]
fn canonical_rules_are_reserved_and_forced_public() {
    let mut config = config(ViewId::private(), vec![]);
    let target = VisibilityTarget {
        name: "RULES.md",
        path: "/.scope/RULES.md",
        kind: VisibilityNodeKind::File,
        reserved: true,
        file_paths_under: vec!["/.scope/RULES.md"],
    };

    let result = toggle_visibility_target(&mut config, target.clone());

    assert!(!result.changed);
    assert_eq!(
        target_visibility(&config, &target),
        ReviewLabel::View(ViewId::public())
    );
    assert_eq!(rule_label(&config, &target), "forced public");
}

#[test]
fn skipped_rule_preserves_ties_duplicates_and_managed_paths() {
    let public = ViewId::public();
    let private = ViewId::private();
    for default in [public.clone(), private.clone()] {
        let config = config(
            default,
            vec![
                ("/docs/**", public.clone()),
                ("/docs", private.clone()),
                ("/docs/**", private.clone()),
                ("/docs/guide.md", public.clone()),
                ("/docs/private/**", private.clone()),
            ],
        );
        for index in 0..config.files.rules.len() {
            let mut removed = config.clone();
            removed.files.rules.remove(index);
            for value in [
                "/",
                "/docs",
                "/docs/guide.md",
                "/docs/private/key",
                "/docs-other",
                "/.scope/RULES.md",
                "/.scope/runs/check.yml",
            ] {
                let path = ScopePath::parse(value).unwrap();
                assert_eq!(
                    config.label_for_path_skipping_rule(&path, Some(index)),
                    removed.label_for_path(&path)
                );
            }
        }
    }
}

#[test]
fn toggling_one_spelling_does_not_expose_another_spelling() {
    use crate::views::{ViewDefinition, ViewIncludes, ViewReaders, Views};
    use std::collections::BTreeSet;

    let label =
        |config: &RepoConfig, path: &str| config.label_for_path(&ScopePath::parse(path).unwrap());

    let mut private_default = config(ViewId::private(), vec![]);
    assert!(toggle_visibility_target(&mut private_default, file("/Secret.txt")).changed);
    assert_eq!(label(&private_default, "/Secret.txt"), ViewId::public());
    assert_eq!(label(&private_default, "/secret.txt"), ViewId::private());
    assert_eq!(
        rule_label(&private_default, &file("/secret.txt")),
        "inherited default"
    );

    let agent = ViewId::parse("agent").unwrap();
    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
    definitions.push(ViewDefinition {
        id: agent.clone(),
        name: "Agent".into(),
        includes: ViewIncludes::Some(BTreeSet::from([ViewId::public()])),
        readers: ViewReaders::Assigned,
    });
    let mut custom_config = config(ViewId::private(), vec![]);
    custom_config.views = Views::new(definitions).unwrap();
    assert!(toggle_visibility_target(&mut custom_config, file("/Secret.txt")).changed);
    assert_eq!(label(&custom_config, "/Secret.txt"), agent);
    assert_eq!(label(&custom_config, "/secret.txt"), ViewId::private());
    assert_eq!(
        rule_label(&custom_config, &file("/secret.txt")),
        "inherited default"
    );

    let mut public_default = config(
        ViewId::public(),
        vec![("/secrets/key.txt", ViewId::private())],
    );
    assert_eq!(
        label(&public_default, "/Secrets/Key.txt"),
        ViewId::private()
    );
    assert_eq!(
        rule_label(&public_default, &file("/Secrets/Key.txt")),
        "inherited /secrets/key.txt"
    );
    assert!(toggle_visibility_target(&mut public_default, file("/Secrets/Key.txt")).changed);
    assert_eq!(label(&public_default, "/Secrets/Key.txt"), ViewId::public());
    assert_eq!(
        label(&public_default, "/secrets/key.txt"),
        ViewId::private()
    );
    assert_eq!(
        label(&public_default, "/SECRETS/KEY.TXT"),
        ViewId::private()
    );

    let team_a = ViewId::parse("team-a").unwrap();
    let team_b = ViewId::parse("team-b").unwrap();
    let mut team_defs = Vec::<ViewDefinition>::from(Views::builtin());
    for (id, name) in [(team_a.clone(), "Team A"), (team_b.clone(), "Team B")] {
        team_defs.push(ViewDefinition {
            id,
            name: name.into(),
            includes: ViewIncludes::Some(BTreeSet::from([ViewId::public()])),
            readers: ViewReaders::Assigned,
        });
    }
    let mut conflict_then_private = RepoConfig::with_default_view(ViewId::public());
    conflict_then_private.views = Views::new(team_defs).unwrap();
    conflict_then_private.files.rules = [
        ("/secrets/**", team_a),
        ("/Secrets/**", team_b),
        ("/SECRETS/**", ViewId::private()),
    ]
    .into_iter()
    .map(|(path, view)| RepoConfigFileRule {
        path: path.to_string(),
        view,
    })
    .collect();
    conflict_then_private.validate().unwrap();
    assert_eq!(
        label(&conflict_then_private, "/sEcReTs/key.txt"),
        ViewId::private()
    );
    assert_eq!(
        rule_label(&conflict_then_private, &file("/sEcReTs/key.txt")),
        "inherited /SECRETS/**"
    );
}

#[test]
fn an_unrelated_edit_or_rule_order_never_changes_another_paths_label() {
    let (public, private) = (ViewId::public(), ViewId::private());
    let cases = [
        (
            private.clone(),
            vec![
                ("/secrets/**", public.clone()),
                ("/SECRETS/**", private.clone()),
            ],
        ),
        (
            public.clone(),
            vec![
                ("/a/**", private.clone()),
                ("/A/**", public.clone()),
                ("/A/b/**", public.clone()),
            ],
        ),
        (
            public.clone(),
            vec![
                ("/a/**", public.clone()),
                ("/A/**", private.clone()),
                ("/A/b/**", private.clone()),
            ],
        ),
        (
            public.clone(),
            vec![
                ("/docs/**", public),
                ("/DOCS/**", private.clone()),
                ("/docs/guide.md", private),
            ],
        ),
    ];
    let probes = [
        "/secrets/key.txt",
        "/SECRETS/key.txt",
        "/Secrets/key.txt",
        "/a/b/x",
        "/A/b/x",
        "/a/B/x",
        "/A/B/x",
        "/docs",
        "/Docs",
        "/docs/guide.md",
        "/Docs/guide.md",
    ];
    let labels = |config: &RepoConfig| {
        probes
            .iter()
            .map(|path| {
                (
                    *path,
                    config.label_for_path(&ScopePath::parse(path).unwrap()),
                )
            })
            .collect::<Vec<_>>()
    };
    for (default, rules) in cases {
        let original = config(default.clone(), rules.clone());
        let reversed = config(default, rules.into_iter().rev().collect());
        let expected = labels(&original);
        assert_eq!(labels(&reversed), expected, "{:?}", original.files.rules);

        for mut edited in [original, reversed] {
            let unrelated = "/unrelated.txt";
            let before = edited.label_for_path(&ScopePath::parse(unrelated).unwrap());
            assert!(toggle_visibility_target(&mut edited, file(unrelated)).changed);
            assert_ne!(
                edited.label_for_path(&ScopePath::parse(unrelated).unwrap()),
                before
            );
            assert_eq!(labels(&edited), expected, "{:?}", edited.files.rules);
        }
    }
}
