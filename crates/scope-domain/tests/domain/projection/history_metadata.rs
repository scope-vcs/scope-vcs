use super::*;
use scope_domain::history::{HistoryNativeCommits, HistoryView, history_view_from_projection};

fn history_view(
    graph: &SourceGraph,
    sets: &[VisibilityChangeSet],
    view_key: ViewId,
) -> HistoryView {
    history_view_from_projection(
        project_graph(graph, sets, &Views::builtin(), &view_key),
        graph,
        sets,
        &Views::builtin(),
        &view_key,
    )
}

#[test]
fn native_history_refs_follow_the_origin_view_and_round_trip() {
    let native = NativeRequestCommit {
        oid: "native".into(),
        parent_oids: vec!["base".into()],
        tree_oid: "tree".into(),
        changed_paths: vec![path("/README.md")],
    };
    let mut logical = commit(
        "merge",
        "Merge request",
        added("/README.md", ViewId::public(), "public"),
    );
    logical.origin = LogicalCommitOrigin::RequestMerge {
        request_id: "request".into(),
        view: ViewId::public(),
        base_oid: "base".into(),
        parent_oids: vec!["base".into()],
        request_head_oid: "native".into(),
        commits: vec![native.clone()],
        preserve_commits: true,
    };
    let mut source = graph(vec![logical]);
    let public = history_view(&source, &[], ViewId::public());
    assert_eq!(
        public.entries[0].native_commits,
        Some(HistoryNativeCommits {
            view: ViewId::public(),
            commits: vec![native.clone()],
        })
    );
    assert_eq!(public.entries[0].message, "Merge request");
    let persisted: HistoryView =
        serde_json::from_slice(&serde_json::to_vec(&public).unwrap()).unwrap();
    assert_eq!(persisted, public);
    source.commits[0]
        .changes
        .push(added("/secret.md", ViewId::private(), "secret"));
    let mixed = history_view(&source, &[], ViewId::public());
    assert!(
        mixed
            .entries
            .iter()
            .all(|entry| entry.native_commits.is_none())
    );
    assert_eq!(
        history_view(&source, &[], ViewId::private()).entries[0].native_commits,
        Some(HistoryNativeCommits {
            view: ViewId::public(),
            commits: vec![native],
        })
    );
    if let LogicalCommitOrigin::RequestMerge {
        preserve_commits, ..
    } = &mut source.commits[0].origin
    {
        *preserve_commits = false;
    }
    source.commits[0].changes.pop();
    assert!(
        history_view(&source, &[], ViewId::public())
            .entries
            .iter()
            .all(|entry| entry.native_commits.is_none())
    );
}
