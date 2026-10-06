use super::*;
use crate::{
    repo_config::RepoConfigFileRule,
    requests::fixtures::{agent, views_with_agent},
};

fn config(default_view: ViewId) -> RepoConfig {
    RepoConfig::with_default_view(default_view)
}

fn agent_config() -> RepoConfig {
    let mut config = config(ViewId::public());
    config.views = views_with_agent();
    config.files.rules = vec![
        RepoConfigFileRule {
            path: "/src/**".into(),
            view: agent(),
        },
        RepoConfigFileRule {
            path: "/ops/**".into(),
            view: ViewId::private(),
        },
    ];
    config.validate().unwrap();
    config
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

#[test]
fn current_view_paths_override_hidden_history_but_never_protected_paths() {
    let config = config(ViewId::private());
    let history = PathHistory {
        visibility_changes: vec![(path("/visible.txt"), ViewId::private(), ViewId::public())],
        ..PathHistory::default()
    };
    let views = config.views.clone();
    let public = ViewId::public();
    let visible = BTreeSet::from(["/visible.txt".into(), "/.scope/repo.json".into()]);
    let paths = RequestViewPaths::new(&config, &views, &public, &visible, &history);
    assert_eq!(paths.ensure_editable(&path("/visible.txt")), Ok(()));
    assert_eq!(
        paths.ensure_editable(&path("/.scope/repo.json")),
        Err(RequestViewPathError::ProtectedPath)
    );
    assert_eq!(
        paths.ensure_editable(&path("/new.txt")),
        Err(RequestViewPathError::HiddenPath)
    );
}

#[test]
fn deleted_or_relabelled_hidden_paths_cannot_be_recreated_under_view_defaults() {
    let config = config(ViewId::public());
    let history = PathHistory {
        file_change_labels: vec![(path("/old-private.txt"), ViewId::private())],
        visibility_changes: vec![(path("/hidden.txt"), ViewId::public(), ViewId::private())],
        ..PathHistory::default()
    };
    let views = config.views.clone();
    let public = ViewId::public();
    let visible = BTreeSet::new();
    let paths = RequestViewPaths::new(&config, &views, &public, &visible, &history);
    for value in ["/old-private.txt", "/hidden.txt"] {
        assert_eq!(
            paths.ensure_editable(&path(value)),
            Err(RequestViewPathError::HiddenPath)
        );
    }
    assert_eq!(paths.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn live_files_hidden_from_the_view_stay_hidden() {
    let config = config(ViewId::public());
    let history = PathHistory {
        live_paths: BTreeSet::from([path("/live-private.txt")]),
        file_change_labels: vec![(path("/always-public.txt"), ViewId::public())],
        ..PathHistory::default()
    };
    let views = config.views.clone();
    let public = ViewId::public();
    let visible = BTreeSet::new();
    let paths = RequestViewPaths::new(&config, &views, &public, &visible, &history);
    assert_eq!(
        paths.ensure_editable(&path("/live-private.txt")),
        Err(RequestViewPathError::HiddenPath)
    );
    assert_eq!(paths.ensure_editable(&path("/always-public.txt")), Ok(()));
}

#[test]
fn a_public_request_cannot_recreate_a_path_that_once_carried_a_custom_label() {
    let config = agent_config();
    let history = PathHistory {
        file_change_labels: vec![(path("/notes.txt"), agent())],
        ..PathHistory::default()
    };
    let views = config.views.clone();
    let public = ViewId::public();
    let visible = BTreeSet::new();
    let paths = RequestViewPaths::new(&config, &views, &public, &visible, &history);
    assert_eq!(
        paths.ensure_editable(&path("/notes.txt")),
        Err(RequestViewPathError::HiddenPath)
    );
    assert_eq!(paths.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn an_agent_request_edits_only_paths_the_agent_view_shows_or_may_create() {
    let config = agent_config();
    let history = PathHistory {
        file_change_labels: vec![
            (path("/old-public.txt"), ViewId::public()),
            (path("/src/old-agent.rs"), agent()),
            (path("/src/once-private.rs"), ViewId::private()),
        ],
        ..PathHistory::default()
    };
    let views = config.views.clone();
    let view = agent();
    let visible = BTreeSet::from(["/src/lib.rs".into()]);
    let paths = RequestViewPaths::new(&config, &views, &view, &visible, &history);
    for editable in [
        "/src/lib.rs",
        "/src/new.rs",
        "/src/old-agent.rs",
        "/old-public.txt",
        "/README.md",
    ] {
        assert_eq!(paths.ensure_editable(&path(editable)), Ok(()), "{editable}");
    }
    for hidden in ["/ops/deploy.sh", "/src/once-private.rs"] {
        assert_eq!(
            paths.ensure_editable(&path(hidden)),
            Err(RequestViewPathError::HiddenPath),
            "{hidden}"
        );
    }
    for protected in ["/.scope/RULES.md", "/AGENTS.md", "/.claude/settings.json"] {
        assert_eq!(
            paths.ensure_editable(&path(protected)),
            Err(RequestViewPathError::ProtectedPath),
            "{protected}"
        );
    }
    assert_eq!(
        paths.rejection(&path("/ops/deploy.sh"), RequestViewPathError::HiddenPath),
        "/ops/deploy.sh is not shown by the Agent view"
    );
    assert_eq!(
        paths.rejection(&path("/AGENTS.md"), RequestViewPathError::ProtectedPath),
        "Agent requests cannot change maintainer-controlled paths: /AGENTS.md"
    );
}
