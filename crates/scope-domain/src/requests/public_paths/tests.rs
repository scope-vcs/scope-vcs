use super::*;
use crate::{
    account::UserAccount,
    projection::{FileChange, LogicalCommit, LogicalCommitOrigin},
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

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

#[test]
fn current_public_paths_override_private_history_but_never_protected_paths() {
    let mut repo = repository(ViewId::private());
    repo.visibility_change_sets.push(
        VisibilityChangeSet::new(
            "visibility".into(),
            None,
            None,
            "owner".into(),
            vec![VisibilityChange {
                path: path("/visible.txt"),
                old_label: ViewId::private(),
                new_label: ViewId::public(),
                current_content: None,
            }],
            None,
        )
        .unwrap(),
    );
    let visible = BTreeSet::from(["/visible.txt".into(), "/.scope/repo.json".into()]);
    let policy = PublicRequestPaths::new(&repo, &visible);
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
    let mut repo = repository(ViewId::public());
    repo.graph.commits.push(LogicalCommit {
        id: "commit".into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "a".repeat(40),
        },
        author_id: "owner".into(),
        message: "delete private source".into(),
        occurred_at_unix: None,
        changes: vec![FileChange {
            path: path("/old-private.txt"),
            old_content: None,
            new_content: None,
            label: ViewId::private(),
        }],
    });
    repo.visibility_change_sets.push(
        VisibilityChangeSet::new(
            "visibility".into(),
            None,
            None,
            "owner".into(),
            vec![VisibilityChange {
                path: path("/hidden.txt"),
                old_label: ViewId::public(),
                new_label: ViewId::private(),
                current_content: None,
            }],
            None,
        )
        .unwrap(),
    );
    let visible = BTreeSet::new();
    let policy = PublicRequestPaths::new(&repo, &visible);
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
    let mut repo = repository(ViewId::public());
    repo.graph.commits.push(LogicalCommit {
        id: "commit".into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: "a".repeat(40),
        },
        author_id: "owner".into(),
        message: "remove an ops file".into(),
        occurred_at_unix: None,
        changes: vec![FileChange {
            path: path("/ops.txt"),
            old_content: None,
            new_content: None,
            label: ViewId::parse("ops").unwrap(),
        }],
    });
    let visible = BTreeSet::new();
    let policy = PublicRequestPaths::new(&repo, &visible);
    assert_eq!(
        policy.ensure_editable(&path("/ops.txt")),
        Err(PublicRequestPathError::PrivatePath)
    );
    assert_eq!(policy.ensure_editable(&path("/new.txt")), Ok(()));
}
