use super::*;

#[test]
fn history_times_follow_visible_metadata_without_changing_projected_revisions() {
    use scope_domain::history::history_view;
    let mut source = graph(vec![commit(
        "private",
        None,
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
