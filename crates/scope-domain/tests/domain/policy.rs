use scope_domain::policy::{LabelRule, Policy, PolicyError, ScopePath, ScopePathError};
use scope_domain::views::{ViewId, Views};

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

#[test]
fn scope_paths_preserve_filename_whitespace_without_relaxing_absolute_paths() {
    for value in ["/ leading.txt", "/trailing.txt ", "/tab\t", "/line\n"] {
        assert_eq!(path(value).as_str(), value);
    }
    assert_eq!(
        ScopePath::parse(" /file"),
        Err(ScopePathError::RelativePath)
    );
    assert_eq!(
        ScopePath::parse("/../file"),
        Err(ScopePathError::InvalidSegment)
    );
    assert_eq!(
        ScopePath::parse("/./file"),
        Err(ScopePathError::InvalidSegment)
    );
    assert_eq!(path("//docs///file").as_str(), "/docs/file");
}

#[test]
fn filename_whitespace_does_not_inherit_another_files_visibility() {
    use scope_domain::repo_config::{RepoConfig, RepoConfigFileRule};

    let mut config = RepoConfig::with_default_view(ViewId::private());
    config.files.rules.push(RepoConfigFileRule {
        path: "/public.txt".to_string(),
        view: ViewId::public(),
    });
    config.validate().unwrap();
    assert_eq!(
        config.label_for_path(&path("/public.txt")),
        ViewId::public()
    );
    for value in [
        "/public.txt ",
        "/public.txt\t",
        "/public.txt\n",
        "/.scope/RULES.md ",
    ] {
        assert_eq!(
            config.label_for_path(&path(value)),
            ViewId::private(),
            "{value:?}"
        );
    }
    assert_eq!(
        config.label_for_path(&path("/.scope/RULES.md")),
        ViewId::public()
    );
}

fn policy_with_private_internal() -> Policy {
    let mut policy = Policy::new(ViewId::public());
    policy
        .add_rule(LabelRule::private(path("/internal")))
        .unwrap();
    policy
}

#[test]
fn private_parent_hides_children() {
    let policy = policy_with_private_internal();
    let path = path("/internal/model.rs");

    assert!(!policy.can_read(&path, &ViewId::public(), &Views::builtin()));
    assert!(policy.can_read(&path, &ViewId::private(), &Views::builtin()));
}

#[test]
fn batched_rules_preserve_visibility_and_last_replacement() {
    let rules = [
        LabelRule::public(path("/docs")),
        LabelRule::private(path("/docs/secrets")),
        LabelRule::public(path("/src/lib.rs")),
        LabelRule::private(path("/src/lib.rs")),
        LabelRule::public(path("/src/main.rs")),
    ];
    let mut sequential = Policy::new(ViewId::private());
    for rule in rules.clone() {
        sequential.add_rule(rule).unwrap();
    }
    let mut batch = Policy::new(ViewId::private());
    batch.add_rules(rules).unwrap();
    assert_eq!(batch.rules(), sequential.rules());
    for file in [
        "/docs/guide.md",
        "/docs/secrets/key",
        "/src/lib.rs",
        "/src/main.rs",
        "/other",
    ] {
        assert_eq!(
            batch.label(&path(file), &Views::builtin()),
            sequential.label(&path(file), &Views::builtin())
        );
    }
    batch.remove_rule(&path("/docs/secrets"));
    assert_eq!(
        batch.label(&path("/docs/secrets/key"), &Views::builtin()),
        ViewId::public()
    );
}

#[test]
fn batch_rejects_private_ancestors_in_either_input_order_without_mutation() {
    for parent in ["/", "/docs", "/docs/private"] {
        let rules = [
            LabelRule::public(path("/docs/private/file")),
            LabelRule::private(path(parent)),
        ];
        for rules in [rules.clone(), [rules[1].clone(), rules[0].clone()]] {
            let mut policy = Policy::new(ViewId::public());
            let error = policy.add_rules(rules).unwrap_err();
            assert_eq!(
                error,
                PolicyError::PublicIsland {
                    child: path("/docs/private/file"),
                    parent: path(parent),
                }
            );
            assert!(policy.rules().is_empty());
        }
    }
}

#[test]
fn ancestor_lookup_respects_segment_boundaries_and_lexical_siblings() {
    let mut policy = Policy::new(ViewId::private());
    policy
        .add_rules([
            LabelRule::private(path("/a")),
            LabelRule::public(path("/a-b")),
            LabelRule::public(path("/ab/child")),
        ])
        .unwrap();
    let before = policy.rules().to_vec();
    assert!(matches!(
        policy.add_rule(LabelRule::public(path("/a/child"))),
        Err(PolicyError::PublicIsland { .. })
    ));
    assert_eq!(policy.rules(), before);
}

#[test]
fn replacing_a_private_rule_with_public_keeps_existing_children_valid() {
    let mut policy = Policy::new(ViewId::private());
    policy.add_rule(LabelRule::private(path("/docs"))).unwrap();
    policy
        .add_rules([
            LabelRule::public(path("/docs")),
            LabelRule::public(path("/docs/guide.md")),
        ])
        .unwrap();
    assert_eq!(
        policy.label(&path("/docs/guide.md"), &Views::builtin()),
        ViewId::public()
    );
}

#[test]
fn split_rules_rejoin_to_the_same_policy_and_labels() {
    let views = Views::builtin();
    let mut policy = Policy::new(ViewId::private());
    policy
        .add_rules([
            LabelRule::public(path("/docs")),
            LabelRule::private(path("/docs/secret.md")),
            LabelRule::public(path("/readme.md")),
        ])
        .unwrap();

    let (directories, files) = policy
        .clone()
        .split_rules(|rule_path| rule_path.as_str().ends_with(".md"));

    assert_eq!(directories.rules(), &[LabelRule::public(path("/docs"))]);
    assert_eq!(
        directories.label(&path("/docs/secret.md"), &views),
        ViewId::public()
    );
    assert_eq!(
        files,
        vec![
            LabelRule::private(path("/docs/secret.md")),
            LabelRule::public(path("/readme.md")),
        ]
    );
    assert_eq!(directories.join_rules(files.into_iter().rev()), policy);
}

#[test]
fn stored_rules_in_any_order_resolve_the_nearest_ancestor_and_last_duplicate() {
    let views = Views::builtin();
    let stored = serde_json::json!({
        "default": "public",
        "rules": [
            {"path": "/docs/private/open.md", "view": "public"},
            {"path": "/docs/private", "view": "private"},
            {"path": "/", "view": "public"},
            {"path": "/docs/private-notes", "view": "private"},
            {"path": "/docs/private/open.md", "view": "private"},
        ],
    });
    let policy: Policy = serde_json::from_value(stored).unwrap();
    for (file, expected) in [
        ("/docs/private/open.md", ViewId::private()),
        ("/docs/private/deep/file.rs", ViewId::private()),
        ("/docs/private-notes/todo.md", ViewId::private()),
        ("/docs/privately.md", ViewId::public()),
        ("/", ViewId::public()),
    ] {
        assert_eq!(policy.label(&path(file), &views), expected, "{file}");
    }
}

fn strings_up_to(alphabet: &[char], length: usize) -> Vec<String> {
    let mut all = vec![String::new()];
    let mut frontier = all.clone();
    for _ in 0..length {
        frontier = frontier
            .iter()
            .flat_map(|prefix| alphabet.iter().map(move |c| format!("{prefix}{c}")))
            .collect();
        all.extend(frontier.iter().cloned());
    }
    all
}

fn assert_scope_path_contract(raw: &str) {
    let segments = raw.split('/').filter(|segment| !segment.is_empty());
    let acceptable = raw.starts_with('/')
        && segments
            .clone()
            .all(|segment| segment != "." && segment != "..");
    match ScopePath::parse(raw) {
        Ok(parsed) => {
            assert!(acceptable, "{raw:?} parsed");
            let text = parsed.as_str();
            assert!(text.starts_with('/'), "{raw:?}");
            assert!(
                text == "/"
                    || text[1..]
                        .split('/')
                        .all(|s| !s.is_empty() && s != "." && s != ".."),
                "{raw:?} -> {text:?}"
            );
            assert_eq!(
                text[1..]
                    .split('/')
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<_>>(),
                segments.collect::<Vec<_>>()
            );
            assert_eq!(ScopePath::parse(parsed.to_string()), Ok(parsed));
        }
        Err(_) => assert!(!acceptable, "{raw:?} rejected"),
    }
}

#[test]
fn scope_path_parsing_is_exhaustively_correct_up_to_seven_characters() {
    for raw in strings_up_to(&['/', '.', 'a', ' '], 7) {
        assert_scope_path_contract(&raw);
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 256, ..Default::default() })]

    #[test]
    fn scope_path_parsing_never_panics_and_round_trips(
        raw in proptest::string::string_regex("(/|\\.|\\.\\.|[a-z]|\\\\|%2e|\\x00|\\PC)*").unwrap()
    ) {
        assert_scope_path_contract(&raw);
    }
}

fn small_paths() -> Vec<ScopePath> {
    let mut paths = vec![ScopePath::root()];
    for depth in 1..=3 {
        let mut level = vec![String::new()];
        for _ in 0..depth {
            level = level
                .iter()
                .flat_map(|prefix| ["a", "ab", "b"].map(|segment| format!("{prefix}/{segment}")))
                .collect();
        }
        paths.extend(level.iter().map(|value| path(value)));
    }
    paths
}

fn nearest_rule_label(rules: &[LabelRule], file: &ScopePath) -> Option<ViewId> {
    let mut candidate = file.as_str().to_string();
    loop {
        if let Some(rule) = rules.iter().find(|rule| rule.path.as_str() == candidate) {
            return Some(rule.view.clone());
        }
        if candidate == "/" {
            return None;
        }
        candidate = match candidate.rfind('/') {
            Some(0) => "/".to_string(),
            Some(index) => candidate[..index].to_string(),
            None => return None,
        };
    }
}

fn island(rules: &[LabelRule]) -> bool {
    rules.iter().any(|child| {
        child.view.is_public()
            && rules.iter().any(|parent| {
                parent.view.is_private()
                    && parent.path != child.path
                    && (parent.path.as_str() == "/"
                        || child
                            .path
                            .as_str()
                            .starts_with(&format!("{}/", parent.path.as_str())))
            })
    })
}

#[test]
fn labels_and_the_public_island_rule_are_exhaustively_correct_for_two_rules() {
    let views = Views::builtin();
    let agent = ViewId::parse("agent").unwrap();
    let paths = small_paths();
    let labels = [ViewId::public(), ViewId::private(), agent];
    let rules = paths
        .iter()
        .flat_map(|rule_path| {
            labels.iter().map(|view| LabelRule {
                path: rule_path.clone(),
                view: view.clone(),
            })
        })
        .collect::<Vec<_>>();
    let existing = Policy::new(ViewId::private());
    for first in &rules {
        for second in &rules {
            let batch = [first.clone(), second.clone()];
            let merged = if first.path == second.path {
                vec![second.clone()]
            } else {
                batch.to_vec()
            };
            let mut policy = existing.clone();
            match policy.add_rules(batch) {
                Ok(()) => {
                    assert!(!island(&merged), "{merged:?} accepted");
                    for file in &paths {
                        assert_eq!(
                            policy.label(file, &views),
                            nearest_rule_label(&merged, file).unwrap_or_else(ViewId::private),
                            "{merged:?} {file}"
                        );
                    }
                }
                Err(PolicyError::PublicIsland { .. }) => {
                    assert!(island(&merged), "{merged:?} rejected");
                    assert_eq!(policy, existing);
                }
            }
        }
    }
}
