mod case;

use case::{Side, Source};
use proptest::{prelude::*, test_runner::Config};
use scope_domain::{
    policy::Policy,
    projection::{FileChange, LogicalCommit},
    projection_views::{
        ProjectionPreviewCommitVisibility, ProjectionPreviewSource, ProjectionPreviewView,
        projection_preview,
    },
    views::{ViewId, Views},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use std::collections::BTreeMap;

fn preview(
    source: &Source,
    reader: &ViewId,
    include_private_counts: bool,
) -> ProjectionPreviewView {
    projection_preview(
        ProjectionPreviewSource {
            repo_id: &source.graph.repo_id,
            policy: &source.policy,
            graph: &source.graph,
            visibility_change_sets: &source.sets,
        },
        &source.current,
        reader,
        include_private_counts,
        &source.native_details,
    )
    .unwrap()
}

fn without_disclosed_hidden_counts(mut preview: ProjectionPreviewView) -> ProjectionPreviewView {
    preview.summary.hidden_files = 0;
    preview.summary.hidden_commits = 0;
    preview
}

proptest! {
    #![proptest_config(Config { cases: 192, ..Config::default() })]

    #[test]
    fn a_preview_cannot_observe_files_it_cannot_read(case in case::case()) {
        let (a, b) = (case.build(Side::A), case.build(Side::B));
        let reader = &case.reader;
        prop_assert_eq!(preview(&a, reader, false), preview(&b, reader, false));
        prop_assert_eq!(
            without_disclosed_hidden_counts(preview(&a, reader, true)),
            without_disclosed_hidden_counts(preview(&b, reader, true))
        );
    }
}

#[test]
fn preview_visibility_of_a_source_update_discloses_whether_it_had_hidden_changes() {
    let reader = ViewId::public();
    let preview_with = |hidden_changes: Vec<FileChange>| {
        let graph = super::graph(vec![LogicalCommit {
            changes: hidden_changes,
            ..super::commit("c0", "hidden", super::added("/h", ViewId::private(), "h"))
        }]);
        let sets = [VisibilityChangeSet {
            occurred_at_unix: None,
            id: "s0".into(),
            anchor_commit_id: None,
            source_update_id: Some("c0".into()),
            author_id: "owner".into(),
            changes: vec![VisibilityChange {
                path: super::path("/README.md"),
                old_label: ViewId::public(),
                new_label: ViewId::private(),
                current_content: None,
            }],
            views: None,
        }];
        let policy = Policy::new(ViewId::public());
        let source = ProjectionPreviewSource {
            repo_id: "scope",
            policy: &policy,
            graph: &graph,
            visibility_change_sets: &sets,
        };
        projection_preview(source, &Views::builtin(), &reader, false, &BTreeMap::new())
            .unwrap()
            .commits[0]
            .visibility
    };
    assert_eq!(
        preview_with(Vec::new()),
        ProjectionPreviewCommitVisibility::AllInView
    );
    assert_eq!(
        preview_with(vec![super::added("/h", ViewId::private(), "h")]),
        ProjectionPreviewCommitVisibility::NoneInView
    );
}
