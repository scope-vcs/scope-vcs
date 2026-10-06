use super::*;
use scope_domain::repo_config::RepoConfigFileRule;

fn id(value: &str) -> ViewId {
    ViewId::parse(value).unwrap()
}

fn parse(args: &[&str]) -> Result<ViewCommand, clap::Error> {
    ViewArgs::try_parse_from(std::iter::once("view").chain(args.iter().copied()))
        .map(|args| args.command)
}

fn edit(args: &[&str]) -> ViewEdit {
    match parse(args).unwrap() {
        ViewCommand::Edit(edit) => edit,
        ViewCommand::List => panic!("expected an edit"),
    }
}

fn add(config: RepoConfig, args: &[&str]) -> RepoConfig {
    apply_view_command(config, &edit(args)).unwrap()
}

fn error(config: RepoConfig, args: &[&str]) -> String {
    apply_view_command(config, &edit(args))
        .unwrap_err()
        .to_string()
}

fn with_agent() -> RepoConfig {
    add(
        RepoConfig::with_default_view(ViewId::private()),
        &["add", "agent", "--name", "Agent", "--include", "public"],
    )
}

#[test]
fn arguments_parse_into_view_edits() {
    assert!(matches!(parse(&["list"]).unwrap(), ViewCommand::List));
    assert_eq!(
        edit(&[
            "add",
            "agent",
            "--name",
            "Agent",
            "--include",
            "public",
            "--anyone"
        ]),
        ViewEdit::Add {
            id: id("agent"),
            name: "Agent".into(),
            includes: vec![ViewId::public()],
            anyone: true,
        }
    );
    assert_eq!(
        edit(&["rename", "agent", "Agents"]),
        ViewEdit::Rename {
            id: id("agent"),
            name: "Agents".into(),
        }
    );
    assert_eq!(
        edit(&["remove", "agent"]),
        ViewEdit::Remove { id: id("agent") }
    );
    assert_eq!(
        edit(&["include", "agent", "--remove", "public"]),
        ViewEdit::Include {
            id: id("agent"),
            change: IncludeChange {
                add: None,
                remove: Some(ViewId::public()),
            },
        }
    );
}

#[test]
fn arguments_reject_bad_ids_and_ambiguous_include_changes() {
    for args in [
        &["add", "Agent", "--name", "Agent"][..],
        &["add", "agent"],
        &["add", "agent", "--name", "Agent", "--include", "Public"],
        &["include", "agent"],
        &["include", "agent", "--add", "public", "--remove", "public"],
    ] {
        assert!(parse(args).is_err(), "{args:?}");
    }
}

#[test]
fn add_appends_an_assigned_view_after_the_builtins() {
    let config = with_agent();
    let agent = config.views().get(&id("agent")).unwrap();
    assert_eq!(agent.name, "Agent");
    assert_eq!(agent.readers, ViewReaders::Assigned);
    assert_eq!(
        agent.includes,
        ViewIncludes::Some([ViewId::public()].into())
    );
    assert_eq!(
        config
            .views()
            .iter()
            .map(|view| view.id.as_str())
            .collect::<Vec<_>>(),
        ["public", "private", "agent"]
    );
}

#[test]
fn rename_changes_only_the_name() {
    let before = with_agent();
    let after = add(before.clone(), &["rename", "agent", "  Agents "]);
    let mut expected = Vec::from(before.views().clone());
    expected[2].name = "Agents".into();
    assert_eq!(Vec::from(after.views().clone()), expected);
    assert_eq!(after.files, before.files);
}

#[test]
fn include_adds_and_removes_included_views() {
    let config = add(with_agent(), &["add", "docs", "--name", "Docs"]);
    let config = add(config, &["include", "agent", "--add", "docs"]);
    assert!(config.views().labels(&id("agent")).contains(&id("docs")));
    let config = add(config, &["include", "agent", "--remove", "public"]);
    assert_eq!(
        config.views().get(&id("agent")).unwrap().includes,
        ViewIncludes::Some([id("docs")].into())
    );
    assert!(
        error(config.clone(), &["include", "agent", "--remove", "public"])
            .contains("does not include public")
    );
    assert!(
        error(config, &["include", "private", "--add", "agent"])
            .contains("already includes every view")
    );
}

#[test]
fn remove_drops_an_unused_view() {
    let config = add(with_agent(), &["remove", "agent"]);
    assert!(config.views().get(&id("agent")).is_none());
}

#[test]
fn domain_validation_refuses_invalid_edits() {
    let config = with_agent();
    assert!(error(config.clone(), &["add", "agent", "--name", "Other"]).contains("unique"));
    assert!(
        error(config.clone(), &["add", "docs", "--name", "agent"]).contains("used more than once")
    );
    assert!(
        error(
            config.clone(),
            &["add", "docs", "--name", "Docs", "--include", "missing"]
        )
        .contains("unknown view missing")
    );
    assert!(
        error(
            config.clone(),
            &["add", "docs", "--name", "Docs", "--anyone"]
        )
        .contains("anyone readers")
    );
    assert!(error(config.clone(), &["remove", "missing"]).contains("Unknown view missing"));
    assert!(error(config.clone(), &["remove", "private"]).contains("private"));
    let referenced = add(
        config,
        &["add", "docs", "--name", "Docs", "--include", "agent"],
    );
    assert!(error(referenced, &["remove", "agent"]).contains("includes unknown view agent"));
}

#[test]
fn removing_a_view_that_labels_files_is_refused() {
    let mut config = with_agent();
    config.files.rules.push(RepoConfigFileRule {
        path: "/src/**".into(),
        view: id("agent"),
    });
    assert!(error(config.clone(), &["remove", "agent"]).contains("relabel"));
    config.files.rules.clear();
    config.files.default = id("agent");
    assert!(error(config, &["remove", "agent"]).contains("relabel"));
}

#[test]
fn a_seventeenth_view_is_refused() {
    let config = (4..=16).fold(with_agent(), |config, index| {
        let view = format!("view{index}");
        add(config, &["add", &view, "--name", &view])
    });
    assert_eq!(config.views().iter().count(), Views::MAX_VIEWS);
    assert!(error(config, &["add", "extra", "--name", "Extra"]).contains("between 1 and 16 views"));
}

#[test]
fn list_shows_names_and_neutralizes_control_characters() {
    let config = add(
        with_agent(),
        &[
            "add",
            "docs",
            "--name",
            "Do\u{1b}[31mcs",
            "--include",
            "agent",
        ],
    );
    assert_eq!(
        view_table(config.views()),
        [
            "ID       NAME       INCLUDES    READERS",
            "public   Public     -           anyone",
            "private  Private    every view  assigned members",
            "agent    Agent      Public      assigned members",
            "docs     Do [31mcs  Agent       assigned members",
        ]
    );
}
