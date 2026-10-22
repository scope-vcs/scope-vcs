use super::*;
use scope_domain::requests::{RequestDiffSide, RequestLineRange};

fn anchor(revision_id: &str, path: Option<&str>, lines: bool) -> RequestDiscussionAnchor {
    RequestDiscussionAnchor {
        revision_id: revision_id.to_string(),
        commit_oid: None,
        path: path.map(|path| ScopePath::parse(path).unwrap()),
        lines: lines.then(|| RequestDiscussionLines {
            side: RequestDiffSide::New,
            range: RequestLineRange::new(1, 2).unwrap(),
        }),
    }
}

#[test]
fn path_anchors_are_inspected_once_per_revision_and_others_need_no_repository() {
    let anchors = [
        ("a", anchor("revision-1", Some("/a.rs"), true)),
        ("b", anchor("revision-1", Some("/b.rs"), true)),
        ("c", anchor("revision-1", Some("/c.rs"), false)),
        ("d", anchor("revision-2", Some("/a.rs"), true)),
        ("e", anchor("revision-2", None, false)),
    ];
    let anchored = anchors
        .iter()
        .map(|(id, anchor)| (*id, anchor))
        .collect::<Vec<_>>();

    let (projections, inspections) =
        plan_inspections(anchored, &BTreeSet::new(), Some("revision-2"), false);

    let grouped = inspections
        .iter()
        .map(|(revision_id, anchors)| {
            let ids = anchors
                .iter()
                .map(|anchor| (anchor.discussion_id.as_str(), anchor.carried.is_some()))
                .collect::<Vec<_>>();
            (revision_id.as_str(), ids)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        grouped,
        [
            ("revision-1", vec![("a", true), ("b", true), ("c", false)]),
            ("revision-2", vec![("d", false)]),
        ]
    );
    assert_eq!(projections.keys().collect::<Vec<_>>(), ["e"]);
}
