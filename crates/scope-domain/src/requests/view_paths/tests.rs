use super::*;
use crate::{
    account::UserAccount,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
    repo_config::RepoConfigFileRule,
    requests::fixtures::{agent, views_with_agent},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};

fn repository(label: ViewId) -> Repository {
    Repository::new(
        &UserAccount {
            id: "owner".into(),
            handle: "owner".into(),
            email: "owner@example.test".into(),
            email_verified: true,
        },
        "repo",
        label,
        "incarnation",
    )
    .unwrap()
}

fn agent_repository() -> Repository {
    let mut repo = repository(ViewId::public());
    repo.repo_config.views = views_with_agent();
    repo.repo_config.files.rules = vec![
        RepoConfigFileRule {
            path: "/src/**".into(),
            view: agent(),
        },
        RepoConfigFileRule {
            path: "/ops/**".into(),
            view: ViewId::private(),
        },
    ];
    repo.repo_config.validate().unwrap();
    repo
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn removal(path_value: &str, label: ViewId) -> LogicalCommit {
    LogicalCommit {
        id: format!("commit{path_value}"),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "a".repeat(40),
        },
        author_id: "owner".into(),
        message: "remove a file".into(),
        occurred_at_unix: None,
        changes: vec![FileChange {
            path: path(path_value),
            old_content: None,
            new_content: None,
            label,
        }],
    }
}

fn relabel(path_value: &str, old_label: ViewId, new_label: ViewId) -> VisibilityChangeSet {
    VisibilityChangeSet::new(
        "visibility".into(),
        None,
        None,
        "owner".into(),
        vec![VisibilityChange {
            path: path(path_value),
            old_label,
            new_label,
            current_content: None,
        }],
        None,
    )
    .unwrap()
}

#[test]
fn current_view_paths_override_hidden_history_but_never_protected_paths() {
    let mut repo = repository(ViewId::private());
    repo.visibility_change_sets
        .push(relabel("/visible.txt", ViewId::private(), ViewId::public()));
    let views = Views::builtin();
    let public = ViewId::public();
    let visible = BTreeSet::from(["/visible.txt".into(), "/.scope/repo.json".into()]);
    let paths = RequestViewPaths::new(&repo, &views, &public, &visible);
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
    let mut repo = repository(ViewId::public());
    repo.graph
        .commits
        .push(removal("/old-private.txt", ViewId::private()));
    repo.visibility_change_sets
        .push(relabel("/hidden.txt", ViewId::public(), ViewId::private()));
    let views = Views::builtin();
    let public = ViewId::public();
    let visible = BTreeSet::new();
    let paths = RequestViewPaths::new(&repo, &views, &public, &visible);
    for value in ["/old-private.txt", "/hidden.txt"] {
        assert_eq!(
            paths.ensure_editable(&path(value)),
            Err(RequestViewPathError::HiddenPath)
        );
    }
    assert_eq!(paths.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn a_public_request_cannot_recreate_a_path_that_once_carried_a_custom_label() {
    let mut repo = agent_repository();
    repo.graph.commits.push(removal("/notes.txt", agent()));
    let views = repo.repo_config.views.clone();
    let public = ViewId::public();
    let visible = BTreeSet::new();
    let paths = RequestViewPaths::new(&repo, &views, &public, &visible);
    assert_eq!(
        paths.ensure_editable(&path("/notes.txt")),
        Err(RequestViewPathError::HiddenPath)
    );
    assert_eq!(paths.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn an_agent_request_edits_only_paths_the_agent_view_shows_or_may_create() {
    let mut repo = agent_repository();
    repo.graph
        .commits
        .push(removal("/old-public.txt", ViewId::public()));
    repo.graph
        .commits
        .push(removal("/src/old-agent.rs", agent()));
    repo.graph
        .commits
        .push(removal("/src/once-private.rs", ViewId::private()));
    let views = repo.repo_config.views.clone();
    let view = agent();
    let visible = BTreeSet::from(["/src/lib.rs".into()]);
    let paths = RequestViewPaths::new(&repo, &views, &view, &visible);
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
