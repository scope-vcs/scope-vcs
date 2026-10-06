use super::*;
use crate::{
    content_ref::ContentRef,
    projection::{
        FileChange, ProjectionCursor, project_graph, project_graph_after, projection_delta_appends,
    },
    visibility_changes::VisibilityChange,
};

fn blob(value: &str) -> SourceBlob {
    SourceBlob {
        content_ref: ContentRef::blob_sha256(value),
        sha256: format!("sha256:{value}"),
        git_oid: format!("git:{value}"),
        git_file_mode: "100644".into(),
        size_bytes: value.len() as u64,
    }
}

fn path(value: &str) -> ScopePath {
    ScopePath::parse(value).unwrap()
}

fn file(name: &str, visibility: ViewId, old: Option<&str>, new: Option<&str>) -> FileChange {
    FileChange {
        path: path(name),
        label: visibility,
        old_content: old.map(blob),
        new_content: new.map(blob),
    }
}

fn commit(id: &str, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: None,
        id: id.into(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.into(),
        },
        author_id: "maintainer".into(),
        message: format!("Content of {id}"),
        changes,
    }
}

fn graph(commits: Vec<LogicalCommit>) -> SourceGraph {
    SourceGraph {
        repo_id: "owner/repo".into(),
        commits,
    }
}

fn visibility(
    id: &str,
    anchor: Option<&str>,
    source: Option<&str>,
    name: &str,
    new_label: ViewId,
    content: Option<&str>,
) -> VisibilityChangeSet {
    VisibilityChangeSet::new(
        id.into(),
        anchor.map(str::to_string),
        source.map(str::to_string),
        "maintainer".into(),
        vec![VisibilityChange {
            path: path(name),
            old_label: if new_label == ViewId::public() {
                ViewId::private()
            } else {
                ViewId::public()
            },
            new_label,
            current_content: content.map(blob),
        }],
    )
    .unwrap()
}

fn sources(view: &HistoryView) -> Vec<&str> {
    view.entries
        .iter()
        .map(|entry| entry.source_id.as_str())
        .collect()
}

#[test]
fn separated_visibility_and_content_fragments_are_one_action_with_exact_diff_bases() {
    let graph = graph(vec![
        commit(
            "first",
            vec![file("/doc", ViewId::public(), None, Some("first public"))],
        ),
        commit(
            "intervening",
            vec![file(
                "/doc",
                ViewId::public(),
                Some("first public"),
                Some("second public"),
            )],
        ),
        commit(
            "push",
            vec![
                file("/other", ViewId::public(), None, Some("public addition")),
                file(
                    "/doc",
                    ViewId::private(),
                    Some("second public"),
                    Some("private edit"),
                ),
            ],
        ),
    ]);
    let sets = vec![visibility(
        "hide",
        Some("first"),
        Some("push"),
        "/doc",
        ViewId::private(),
        Some("private edit"),
    )];
    let public = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&public), ["push", "intervening", "first"]);
    let push = &public.entries[0];
    assert_eq!(push.parent_id.as_deref(), Some("intervening"));
    assert_eq!(push.files.len(), 1);
    assert_eq!(push.files[0].path, path("/other"));
    assert_eq!(push.visibility_changes.len(), 1);
    let boundary = push.visibility_changes[0].file.as_ref().unwrap();
    assert_eq!(boundary.old_content, Some(blob("first public")));
    assert_eq!(boundary.new_content, None);
    assert_eq!(boundary.kind, FileChangeKind::Deleted);
    assert_eq!(public.entries[1].files[0].old_content, None);
    assert_eq!(
        public.entries[1].files[0].new_content,
        Some(blob("second public"))
    );
    assert_eq!(push.author, None);
    assert_eq!(push.message, "Projected public update");
    assert!(
        !serde_json::to_string(&public)
            .unwrap()
            .contains("private edit")
    );

    let private = history_view(&graph, &sets, &Views::builtin(), &ViewId::private());
    assert_eq!(sources(&private), ["push", "intervening", "first"]);
    assert_eq!(private.entries[0].files.len(), 2);
    assert!(private.entries[0].visibility_changes[0].file.is_none());
    assert_eq!(
        private.entries[0].files[1].old_content,
        Some(blob("second public"))
    );
}

#[test]
fn repeated_path_transitions_are_not_collapsed_to_a_net_change() {
    let graph = graph(vec![
        commit(
            "private-base",
            vec![file("/doc", ViewId::private(), None, Some("baseline"))],
        ),
        commit(
            "push",
            vec![file("/other", ViewId::public(), None, Some("code"))],
        ),
    ]);
    let sets = vec![
        visibility(
            "publish",
            Some("private-base"),
            Some("push"),
            "/doc",
            ViewId::public(),
            Some("baseline"),
        ),
        visibility(
            "hide",
            Some("private-base"),
            Some("push"),
            "/doc",
            ViewId::private(),
            Some("baseline"),
        ),
    ];
    let view = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&view), ["push"]);
    let entry = &view.entries[0];
    assert_eq!(entry.files.len(), 1);
    assert_eq!(entry.visibility_changes.len(), 2);
    let published = &entry.visibility_changes[0];
    let hidden = &entry.visibility_changes[1];
    assert_ne!(published.id, hidden.id);
    assert_eq!(published.path, hidden.path);
    assert_eq!(
        published.file.as_ref().unwrap().new_content,
        Some(blob("baseline"))
    );
    assert_eq!(
        hidden.file.as_ref().unwrap().old_content,
        Some(blob("baseline"))
    );
    assert_eq!(hidden.new_label, ViewId::private());
}

#[test]
fn standalone_actions_stay_between_their_source_anchors_even_when_the_anchor_is_private() {
    let graph = graph(vec![
        commit(
            "private-base",
            vec![file("/doc", ViewId::private(), None, Some("baseline"))],
        ),
        commit(
            "push",
            vec![file("/other", ViewId::public(), None, Some("code"))],
        ),
    ]);
    let sets = vec![visibility(
        "publish",
        Some("private-base"),
        None,
        "/doc",
        ViewId::public(),
        Some("baseline"),
    )];
    let public = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&public), ["push", "publish"]);
    assert_eq!(public.entries[0].parent_id.as_deref(), Some("publish"));
    assert_eq!(public.entries[1].parent_id, None);
    let publish = &public.entries[1];
    assert_eq!(publish.kind, HistoryEntryKind::VisibilityChange);
    assert!(publish.files.is_empty());
    assert_eq!(publish.message, "Made 1 file public");
    assert_eq!(publish.author, None);
    assert_eq!(
        publish.visibility_changes[0].file.as_ref().unwrap().kind,
        FileChangeKind::Added
    );

    let private = history_view(&graph, &sets, &Views::builtin(), &ViewId::private());
    assert_eq!(sources(&private), ["push", "publish", "private-base"]);
    assert_eq!(private.entries[1].author.as_deref(), Some("maintainer"));
    assert!(private.entries[1].visibility_changes[0].file.is_none());
}

#[test]
fn a_push_can_have_visibility_effects_without_any_visible_content_changes() {
    let mut graph = graph(vec![
        commit(
            "base",
            vec![file("/doc", ViewId::private(), None, Some("baseline"))],
        ),
        commit(
            "private-push",
            vec![file("/private", ViewId::private(), None, Some("hidden"))],
        ),
    ]);
    graph.commits[1].occurred_at_unix = Some(1_700_000_000);
    let sets = vec![visibility(
        "publish",
        Some("base"),
        Some("private-push"),
        "/doc",
        ViewId::public(),
        Some("baseline"),
    )];
    let view = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&view), ["private-push"]);
    assert!(view.entries[0].files.is_empty());
    assert_eq!(view.entries[0].kind, HistoryEntryKind::Push);
    assert_eq!(view.entries[0].message, "Projected public update");
    assert_eq!(view.entries[0].author, None);
    assert_eq!(view.entries[0].occurred_at_unix, None);
    let private = history_view(&graph, &sets, &Views::builtin(), &ViewId::private());
    assert_eq!(private.entries[0].occurred_at_unix, Some(1_700_000_000));
    assert!(view.entries[0].visibility_changes[0].file.is_some());
}

#[test]
fn a_file_changed_and_made_public_keeps_its_content_diff_and_transition() {
    let graph = graph(vec![
        commit(
            "base",
            vec![file("/doc", ViewId::private(), None, Some("old private"))],
        ),
        commit(
            "push",
            vec![file(
                "/doc",
                ViewId::public(),
                Some("old private"),
                Some("new public"),
            )],
        ),
    ]);
    let sets = vec![visibility(
        "publish",
        Some("base"),
        Some("push"),
        "/doc",
        ViewId::public(),
        Some("new public"),
    )];
    let view = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&view), ["push"]);
    let entry = &view.entries[0];
    assert_eq!(entry.files.len(), 1);
    assert_eq!(entry.files[0].old_content, None);
    assert_eq!(entry.files[0].new_content, Some(blob("new public")));
    assert_eq!(entry.visibility_changes.len(), 1);
    assert!(entry.visibility_changes[0].file.is_none());
    assert!(
        !serde_json::to_string(&view)
            .unwrap()
            .contains("old private")
    );
}

#[test]
fn private_events_and_empty_public_deletions_do_not_disclose_paths() {
    let graph = graph(vec![commit(
        "base",
        vec![file("/private", ViewId::private(), None, Some("secret"))],
    )]);
    let sets = vec![visibility(
        "hide",
        Some("base"),
        None,
        "/private",
        ViewId::private(),
        Some("secret"),
    )];
    let public = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert!(public.entries.is_empty());
    let private = history_view(&graph, &sets, &Views::builtin(), &ViewId::private());
    assert_eq!(sources(&private), ["hide", "base"]);
    assert_eq!(private.entries[0].visibility_changes.len(), 1);
}

#[test]
fn unresolved_sources_are_standalone_actions_and_missing_anchors_precede_the_graph() {
    let graph = graph(vec![commit(
        "push",
        vec![file("/doc", ViewId::public(), None, Some("code"))],
    )]);
    let sets = vec![visibility(
        "publish",
        Some("missing"),
        Some("missing"),
        "/baseline",
        ViewId::public(),
        Some("baseline"),
    )];
    let view = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&view), ["push", "publish"]);
    assert_eq!(view.entries[1].kind, HistoryEntryKind::VisibilityChange);
    assert_eq!(view.entries[1].parent_id, None);
}

#[test]
fn preview_identity_survives_unrelated_earlier_projection_insertions() {
    let mut graph = graph(vec![commit(
        "base",
        vec![file("/doc", ViewId::private(), None, Some("baseline"))],
    )]);
    let sets = vec![visibility(
        "publish",
        Some("base"),
        None,
        "/doc",
        ViewId::public(),
        Some("baseline"),
    )];
    let before = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    graph.commits.insert(
        0,
        commit(
            "earlier",
            vec![file("/other", ViewId::public(), None, Some("earlier"))],
        ),
    );
    let after = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(
        before.entries[0].visibility_changes[0].id,
        after.entries[0].visibility_changes[0].id
    );
    assert_ne!(before.generation, after.generation);
}

#[test]
fn generation_includes_visibility_preview_content() {
    let graph = graph(vec![]);
    let mut sets = vec![visibility(
        "publish",
        None,
        None,
        "/doc",
        ViewId::public(),
        Some("one"),
    )];
    let before = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    sets[0].changes[0].current_content = Some(blob("two"));
    let after = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_ne!(before.generation, after.generation);
}

#[test]
fn same_blob_with_a_mode_change_is_a_content_change() {
    let mut executable = blob("script");
    executable.git_file_mode = "100755".into();
    let mut second = file("/script", ViewId::public(), Some("script"), Some("script"));
    second.new_content = Some(executable.clone());
    let graph = graph(vec![
        commit(
            "base",
            vec![file("/script", ViewId::public(), None, Some("script"))],
        ),
        commit("mode", vec![second]),
    ]);
    let view = history_view(&graph, &[], &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&view), ["mode", "base"]);
    assert_eq!(view.entries[0].files[0].new_content, Some(executable));
}

#[test]
fn visible_occurrence_time_changes_history_generation_without_changing_projection() {
    let mut graph = graph(vec![commit(
        "push",
        vec![file("/doc", ViewId::public(), None, Some("body"))],
    )]);
    graph.commits[0].occurred_at_unix = Some(1_700_000_000);
    for audience in [ViewId::public(), ViewId::private()] {
        let before = history_view(&graph, &[], &Views::builtin(), &audience);
        let projection = project_graph(&graph, &[], &Views::builtin(), &audience);
        assert_eq!(before.entries[0].occurred_at_unix, Some(1_700_000_000));
        graph.commits[0].occurred_at_unix = Some(1_800_000_000);
        let after = history_view(&graph, &[], &Views::builtin(), &audience);
        assert_eq!(after.entries[0].occurred_at_unix, Some(1_800_000_000));
        assert_ne!(before.generation, after.generation);
        assert_eq!(
            project_graph(&graph, &[], &Views::builtin(), &audience),
            projection
        );
        graph.commits[0].occurred_at_unix = Some(1_700_000_000);
    }
}

#[test]
fn standalone_visibility_time_is_recorded_privately_and_redacted_publicly() {
    let graph = graph(vec![commit(
        "push",
        vec![file("/doc", ViewId::private(), None, Some("body"))],
    )]);
    let mut sets = vec![visibility(
        "publish",
        Some("push"),
        None,
        "/doc",
        ViewId::public(),
        Some("body"),
    )];
    sets[0].occurred_at_unix = Some(1_700_000_000);
    let public = history_view(&graph, &sets, &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&public), ["publish"]);
    assert_eq!(public.entries[0].author, None);
    assert_eq!(public.entries[0].occurred_at_unix, None);
    let private = history_view(&graph, &sets, &Views::builtin(), &ViewId::private());
    assert_eq!(sources(&private), ["publish", "push"]);
    assert_eq!(private.entries[0].occurred_at_unix, Some(1_700_000_000));
    sets[0].occurred_at_unix = Some(1_800_000_000);
    assert_eq!(
        history_view(&graph, &sets, &Views::builtin(), &ViewId::public()),
        public
    );
    assert_ne!(
        history_view(&graph, &sets, &Views::builtin(), &ViewId::private()).generation,
        private.generation
    );
}

#[test]
fn partial_public_push_does_not_disclose_its_occurrence_time() {
    let mut graph = graph(vec![commit(
        "push",
        vec![
            file("/public", ViewId::public(), None, Some("visible")),
            file("/secret", ViewId::private(), None, Some("hidden")),
        ],
    )]);
    graph.commits[0].occurred_at_unix = Some(1_700_000_000);
    let public = history_view(&graph, &[], &Views::builtin(), &ViewId::public());
    assert_eq!(sources(&public), ["push"]);
    assert_eq!(public.entries[0].author, None);
    assert_eq!(public.entries[0].occurred_at_unix, None);
    graph.commits[0].occurred_at_unix = Some(1_800_000_000);
    assert_eq!(
        history_view(&graph, &[], &Views::builtin(), &ViewId::public()),
        public
    );
}

#[test]
fn history_folded_in_steps_matches_history_folded_at_once() {
    let commits = vec![
        commit(
            "first",
            vec![
                file("/doc", ViewId::public(), None, Some("doc")),
                file("/secret", ViewId::private(), None, Some("secret")),
            ],
        ),
        commit(
            "second",
            vec![file("/doc", ViewId::public(), Some("doc"), Some("doc two"))],
        ),
        commit(
            "third",
            vec![
                file("/secret", ViewId::public(), Some("secret"), Some("shared")),
                file(
                    "/doc",
                    ViewId::private(),
                    Some("doc two"),
                    Some("doc three"),
                ),
            ],
        ),
        commit(
            "fourth",
            vec![file("/new", ViewId::public(), None, Some("new"))],
        ),
    ];
    let sets = vec![
        visibility(
            "hide",
            Some("second"),
            Some("third"),
            "/doc",
            ViewId::private(),
            Some("doc three"),
        ),
        visibility(
            "show",
            Some("fourth"),
            None,
            "/doc",
            ViewId::public(),
            Some("doc three"),
        ),
    ];
    let whole = graph(commits.clone());
    let folded_commits = 2;
    let first = graph(commits[..folded_commits].to_vec());
    let rest = graph(commits[folded_commits..].to_vec());
    assert!(projection_delta_appends(
        Some("second"),
        &rest.commits,
        &sets,
        &Views::builtin(),
        &ViewId::public()
    ));
    assert!(!projection_delta_appends(
        Some("first"),
        &rest.commits,
        &sets,
        &Views::builtin(),
        &ViewId::public()
    ));

    for view_key in [ViewId::private(), ViewId::public()] {
        let at_once = project_graph(&whole, &sets, &Views::builtin(), &view_key);
        let prefix = project_graph(&first, &[], &Views::builtin(), &view_key);
        let cursor = ProjectionCursor {
            commit_count: prefix.commits.len(),
            last_projected_id: prefix
                .commits
                .last()
                .map(|commit| commit.projected_id.clone()),
        };
        let suffix = project_graph_after(&cursor, &rest, &sets, &Views::builtin(), &view_key);
        assert_eq!(
            [prefix.commits.clone(), suffix.commits.clone()].concat(),
            at_once.commits
        );

        let expected =
            history_view_from_projection(at_once, &whole, &sets, &Views::builtin(), &view_key);
        let mut history = HistoryCursor::start("owner/repo", &Views::builtin(), &view_key);
        let mut tree = BTreeMap::new();
        prefix.apply_to(&mut tree);
        let mut entries = history_entries_after(
            &mut history,
            BTreeMap::new(),
            prefix,
            &first,
            &[],
            &Views::builtin(),
            &view_key,
        );
        entries.extend(history_entries_after(
            &mut history,
            tree,
            suffix,
            &rest,
            &sets,
            &Views::builtin(),
            &view_key,
        ));
        entries.reverse();
        assert_eq!(entries, expected.entries);
        assert_eq!(history.generation, expected.generation);
        assert_eq!(
            history.last_entry_id.as_deref(),
            expected.entries.first().map(|entry| entry.id.as_str())
        );
    }
}
