use super::*;

#[test]
fn history_times_follow_visible_metadata_without_changing_projected_revisions() {
    use scope_domain::history::history_view;
    let mut source = graph(vec![commit(
        "private",
        "Private history",
        added("/secret.md", Visibility::Private, "secret"),
    )]);
    source.commits[0].occurred_at_unix = Some(1_600_000_000);
    let mut reveal = visibility_event(
        "reveal",
        Some("private"),
        None,
        "/secret.md",
        Visibility::Public,
        blob("secret"),
    );
    reveal.occurred_at_unix = Some(1_700_000_000);
    let sets = vec![reveal];
    let projected = project_graph(&source, &sets, ProjectionViewKey::Public);
    let public = history_view(&source, &sets, ProjectionViewKey::Public);
    assert!(!public.entries.is_empty());
    assert!(
        public
            .entries
            .iter()
            .all(|entry| entry.occurred_at_unix != Some(1_600_000_000))
    );
    assert!(
        public
            .entries
            .iter()
            .filter(|entry| entry.author.is_none())
            .all(|entry| entry.occurred_at_unix.is_none())
    );
    let private = history_view(&source, &sets, ProjectionViewKey::Private);
    assert_eq!(private.entries[0].occurred_at_unix, Some(1_700_000_000));
    assert_eq!(private.entries[1].occurred_at_unix, Some(1_600_000_000));
    source.commits[0].occurred_at_unix = Some(1_650_000_000);
    assert_eq!(
        project_graph(&source, &sets, ProjectionViewKey::Public),
        projected
    );
    assert_eq!(
        history_view(&source, &sets, ProjectionViewKey::Public),
        public
    );
    assert_ne!(
        history_view(&source, &sets, ProjectionViewKey::Private).generation,
        private.generation
    );
}

#[test]
fn native_history_refs_follow_audience_and_round_trip() {
    use scope_domain::history::{HistoryView, history_view};
    let native = NativePublicCommit {
        oid: "native".into(),
        parent_oids: vec!["base".into()],
        tree_oid: "tree".into(),
        changed_paths: vec![path("/README.md")],
    };
    let mut logical = commit(
        "merge",
        "Merge request",
        added("/README.md", Visibility::Public, "public"),
    );
    logical.origin = LogicalCommitOrigin::PublicRequestMerge {
        request_id: "request".into(),
        public_base_oid: "base".into(),
        public_parent_oids: vec!["base".into()],
        request_head_oid: "native".into(),
        commits: vec![native.clone()],
        preserve_public_commits: true,
    };
    let mut source = graph(vec![logical]);
    let public = history_view(&source, &[], ProjectionViewKey::Public);
    assert_eq!(public.entries[0].native_commits, vec![native.clone()]);
    assert_eq!(public.entries[0].message, "Merge request");
    let persisted: HistoryView =
        serde_json::from_slice(&serde_json::to_vec(&public).unwrap()).unwrap();
    assert_eq!(persisted, public);
    source.commits[0]
        .changes
        .push(added("/secret.md", Visibility::Private, "secret"));
    let mixed = history_view(&source, &[], ProjectionViewKey::Public);
    assert!(
        mixed
            .entries
            .iter()
            .all(|entry| entry.native_commits.is_empty())
    );
    assert_eq!(
        history_view(&source, &[], ProjectionViewKey::Private).entries[0].native_commits,
        vec![native]
    );
    if let LogicalCommitOrigin::PublicRequestMerge {
        preserve_public_commits,
        ..
    } = &mut source.commits[0].origin
    {
        *preserve_public_commits = false;
    }
    source.commits[0].changes.pop();
    assert!(
        history_view(&source, &[], ProjectionViewKey::Public)
            .entries
            .iter()
            .all(|entry| entry.native_commits.is_empty())
    );
}
