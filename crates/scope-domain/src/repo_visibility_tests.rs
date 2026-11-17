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
fn toggling_a_file_leaves_a_rule_written_in_another_case_in_force() {
    let mut edited = config(
        ViewId::public(),
        vec![("/secrets/key.txt", ViewId::private())],
    );
    let label =
        |config: &RepoConfig, path: &str| config.label_for_path(&ScopePath::parse(path).unwrap());
    assert_eq!(label(&edited, "/Secrets/Key.txt"), ViewId::private());

    assert!(toggle_visibility_target(&mut edited, file("/Secrets/Key.txt")).changed);

    assert_eq!(label(&edited, "/Secrets/Key.txt"), ViewId::public());
    assert_eq!(label(&edited, "/secrets/key.txt"), ViewId::private());
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
