use super::*;

fn config(default_view: ViewId) -> RepoConfig {
    RepoConfig::with_default_view(default_view)
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

#[test]
fn current_public_paths_override_private_history_but_never_protected_paths() {
    let config = config(ViewId::private());
    let history = PathHistory {
        visibility_changes: vec![(path("/visible.txt"), ViewId::private(), ViewId::public())],
        ..PathHistory::default()
    };
    let visible = BTreeSet::from(["/visible.txt".into(), "/.scope/repo.json".into()]);
    let policy = PublicRequestPaths::new(&config, &visible, &history);
    assert_eq!(policy.ensure_editable(&path("/visible.txt")), Ok(()));
    assert_eq!(
        policy.ensure_editable(&path("/.scope/repo.json")),
        Err(PublicRequestPathError::ProtectedPath)
    );
    assert_eq!(
        policy.ensure_editable(&path("/new.txt")),
        Err(PublicRequestPathError::PrivatePath)
    );
}

#[test]
fn deleted_or_renamed_private_paths_cannot_be_recreated_under_public_defaults() {
    let config = config(ViewId::public());
    let history = PathHistory {
        file_change_labels: vec![(path("/old-private.txt"), ViewId::private())],
        visibility_changes: vec![(path("/hidden.txt"), ViewId::public(), ViewId::private())],
        ..PathHistory::default()
    };
    let visible = BTreeSet::new();
    let policy = PublicRequestPaths::new(&config, &visible, &history);
    for value in ["/old-private.txt", "/hidden.txt"] {
        assert_eq!(
            policy.ensure_editable(&path(value)),
            Err(PublicRequestPathError::PrivatePath)
        );
    }
    assert_eq!(policy.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn paths_once_labelled_with_a_custom_view_are_hidden_history_too() {
    let config = config(ViewId::public());
    let history = PathHistory {
        file_change_labels: vec![(path("/ops.txt"), ViewId::parse("ops").unwrap())],
        ..PathHistory::default()
    };
    let visible = BTreeSet::new();
    let policy = PublicRequestPaths::new(&config, &visible, &history);
    assert_eq!(
        policy.ensure_editable(&path("/ops.txt")),
        Err(PublicRequestPathError::PrivatePath)
    );
    assert_eq!(policy.ensure_editable(&path("/new.txt")), Ok(()));
}

#[test]
fn live_files_hidden_from_the_public_view_stay_private() {
    let config = config(ViewId::public());
    let history = PathHistory {
        live_paths: BTreeSet::from([path("/live-private.txt")]),
        file_change_labels: vec![(path("/always-public.txt"), ViewId::public())],
        ..PathHistory::default()
    };
    let visible = BTreeSet::new();
    let policy = PublicRequestPaths::new(&config, &visible, &history);
    assert_eq!(
        policy.ensure_editable(&path("/live-private.txt")),
        Err(PublicRequestPathError::PrivatePath)
    );
    assert_eq!(policy.ensure_editable(&path("/always-public.txt")), Ok(()));
}
