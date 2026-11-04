mod policy;
mod views;

use super::{blob, graph, path};
use scope_domain::{
    content::SourceBlob,
    policy::{LabelRule, Policy},
    projection::{
        FileChange, LabelledTree, LogicalCommit, LogicalCommitOrigin, NativeRequestCommit,
        NativeRequestCommitDetails, Projection, ProjectionCursor, ProjectionMaterialization,
        initial_views, project_graph, project_graph_after,
    },
    projection_views::{
        ProjectionPreviewCommitVisibility, ProjectionPreviewSource, ProjectionPreviewView,
        projection_preview,
    },
    views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views, ViewsTransition},
    visibility_changes::{VisibilityChange, VisibilityChangeSet},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
    process::Command,
};

const CASES: usize = 2000;
const LABELS: [&str; 5] = ["public", "private", "c0", "c1", "unknown"];
const PATHS: [&str; 7] = [
    "/README.md",
    "/src/a.rs",
    "/ops/b.txt",
    "/notes.txt",
    "/.scope",
    "/.scope/runs/x.yml",
    "/.scope/RULES.md",
];

pub(super) struct Rng(pub(super) u64);

impl Rng {
    pub(super) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(super) fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    pub(super) fn chance(&mut self) -> bool {
        self.next() & 1 == 1
    }

    pub(super) fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn file(rng: &mut Rng) -> SourceBlob {
    let mut file = blob(&format!("v{}", rng.below(3)));
    file.git_file_mode = rng.pick(&["100644", "100755", "120000"]).to_string();
    file
}

fn blob_key(blob: &SourceBlob) -> String {
    serde_json::to_string(blob).unwrap()
}

pub(super) fn id(value: &str) -> ViewId {
    ViewId::parse(value).unwrap()
}

fn views(rng: &mut Rng) -> Views {
    let candidates = ["public", "private", "c0", "c1"];
    loop {
        let includes = |mask: usize| -> BTreeSet<ViewId> {
            (0..candidates.len())
                .filter(|bit| mask & (1 << bit) != 0)
                .map(|bit| id(candidates[bit]))
                .collect()
        };
        let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
        definitions[0].includes = ViewIncludes::Some(includes(rng.below(16)));
        for index in 0..rng.below(3) {
            definitions.push(ViewDefinition {
                id: id(&format!("c{index}")),
                name: format!("C{index}"),
                includes: ViewIncludes::Some(includes(rng.below(16))),
                readers: ViewReaders::Assigned,
            });
        }
        if let Ok(views) = Views::new(definitions) {
            return views;
        }
    }
}

fn origin(rng: &mut Rng, index: usize, view_ids: &[ViewId]) -> LogicalCommitOrigin {
    match rng.below(3) {
        0 => LogicalCommitOrigin::CanonicalPush {
            source_head_oid: format!("head{index}"),
        },
        1 => LogicalCommitOrigin::PrivateRequestMerge {
            request_id: format!("r{index}"),
            request_head_oid: format!("head{index}"),
        },
        _ => LogicalCommitOrigin::RequestMerge {
            request_id: format!("r{index}"),
            view: rng.pick(view_ids).clone(),
            base_oid: format!("base{index}"),
            parent_oids: vec![format!("base{index}")],
            request_head_oid: format!("head{index}"),
            preserve_commits: rng.chance(),
            commits: (0..rng.below(4))
                .map(|step| NativeRequestCommit {
                    oid: format!("n{index}-{step}"),
                    parent_oids: (0..rng.below(4))
                        .map(|parent| format!("p{index}-{step}-{parent}"))
                        .collect(),
                    tree_oid: format!("t{index}-{step}"),
                    changed_paths: Vec::new(),
                })
                .collect(),
        },
    }
}

fn history(rng: &mut Rng, view_ids: &[ViewId]) -> Vec<LogicalCommit> {
    (0..rng.below(7))
        .map(|index| LogicalCommit {
            occurred_at_unix: None,
            id: format!("c{}", index.saturating_sub(usize::from(rng.below(8) == 0))),
            origin: origin(rng, index, view_ids),
            author_id: format!("author{}", rng.below(2)),
            message: format!("message{}", rng.below(3)),
            changes: (0..rng.below(5))
                .map(|_| FileChange {
                    path: path(rng.pick(&PATHS)),
                    old_content: rng.chance().then(|| file(rng)),
                    new_content: rng.chance().then(|| file(rng)),
                    label: rng.pick(view_ids).clone(),
                })
                .collect(),
        })
        .collect()
}

fn label(rng: &mut Rng) -> ViewId {
    id(rng.pick(&LABELS))
}

fn change_sets(
    rng: &mut Rng,
    commits: &[LogicalCommit],
    mut views: Views,
) -> (Vec<VisibilityChangeSet>, Views) {
    let commit_id = |rng: &mut Rng| match rng.below(4) {
        0 => None,
        1 => Some("missing".to_string()),
        _ => (!commits.is_empty()).then(|| rng.pick(commits).id.clone()),
    };
    let mut sets = Vec::new();
    for index in 0..rng.below(5) {
        let anchor = commit_id(rng);
        let source_update = if rng.chance() { commit_id(rng) } else { None };
        let source_paths = commits
            .iter()
            .filter(|commit| source_update.as_ref() == Some(&commit.id))
            .flat_map(|commit| commit.changes.iter().map(|change| change.path.clone()))
            .collect::<Vec<_>>();
        let mut paths = BTreeSet::new();
        let changes = (0..rng.below(4))
            .filter_map(|_| {
                let (old_label, new_label) = (label(rng), label(rng));
                let path = if !source_paths.is_empty() && rng.chance() {
                    rng.pick(&source_paths).clone()
                } else {
                    path(rng.pick(&PATHS))
                };
                (old_label != new_label && paths.insert(path.clone())).then(|| VisibilityChange {
                    path,
                    old_label,
                    new_label,
                    current_content: rng.chance().then(|| file(rng)),
                })
            })
            .collect::<Vec<_>>();
        let transition = (rng.below(3) == 0).then(|| {
            let after = loop {
                let candidate = self::views(rng);
                if candidate != views {
                    break candidate;
                }
            };
            ViewsTransition {
                before: std::mem::replace(&mut views, after.clone()),
                after,
            }
        });
        if let Ok(set) = VisibilityChangeSet::new(
            format!("s{index}"),
            anchor,
            source_update,
            "owner".into(),
            changes,
            transition,
        ) {
            sets.push(set);
        }
    }
    (sets, views)
}

pub(super) fn views_json(views: &Views) -> Value {
    json!(
        views
            .iter()
            .map(|definition| json!({
                "id": definition.id.as_str(),
                "includes": match &definition.includes {
                    ViewIncludes::All => Value::Null,
                    ViewIncludes::Some(included) => json!(included.iter().map(ViewId::as_str).collect::<Vec<_>>()),
                },
                "readers": definition.readers,
            }))
            .collect::<Vec<_>>()
    )
}

fn set_json(set: &VisibilityChangeSet) -> Value {
    json!({
        "id": set.id,
        "anchor": set.anchor_commit_id,
        "source": set.source_update_id,
        "relabels": set.changes.iter().map(|change| json!({
            "path": change.path.as_str(),
            "old": change.old_label.as_str(),
            "new": change.new_label.as_str(),
            "content": change.current_content.as_ref().map(blob_key),
        })).collect::<Vec<_>>(),
        "views": set.views.as_ref().map(|transition| json!({
            "before": views_json(&transition.before),
            "after": views_json(&transition.after),
        })),
    })
}

fn origin_json(origin: &LogicalCommitOrigin) -> Value {
    match origin {
        LogicalCommitOrigin::RequestMerge {
            view,
            preserve_commits,
            commits,
            ..
        } => json!({
            "kind": "merge",
            "view": view.as_str(),
            "preserve": preserve_commits,
            "native": commits.iter().map(|native| json!({
                "oid": native.oid,
                "parents": native.parent_oids,
                "tree": native.tree_oid,
            })).collect::<Vec<_>>(),
        }),
        _ => json!({ "kind": "push" }),
    }
}

struct Case {
    policy: Policy,
    native: BTreeMap<String, NativeRequestCommitDetails>,
    current: Views,
    reader: ViewId,
    commits: Vec<LogicalCommit>,
    sets: Vec<VisibilityChangeSet>,
    prefix_commits: usize,
    prefix_sets: usize,
}

impl Case {
    fn json(&self) -> Value {
        json!({
            "kind": "projection",
            "policy": self.policy,
            "native": self.native.iter().map(|(oid, details)| json!({
                "oid": oid,
                "author": details.author,
                "message": details.message,
                "changes": details.changes.len(),
            })).collect::<Vec<_>>(),
            "views": views_json(&self.current),
            "reader": self.reader.as_str(),
            "commits": self.commits.iter().map(|commit| json!({
                "id": commit.id,
                "origin": origin_json(&commit.origin),
                "author": commit.author_id,
                "message": commit.message,
                "changes": commit.changes.iter().map(|change| json!({
                    "path": change.path.as_str(),
                    "label": change.label.as_str(),
                    "content": change.new_content.as_ref().map(blob_key),
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "repo_id": graph(Vec::new()).repo_id,
            "sets": self.sets.iter().map(set_json).collect::<Vec<_>>(),
            "prefix_commits": self.prefix_commits,
            "prefix_sets": self.prefix_sets,
        })
    }

    fn rust(&self) -> Value {
        let whole = graph(self.commits.clone());
        let prefix_graph = graph(self.commits[..self.prefix_commits].to_vec());
        let suffix_graph = graph(self.commits[self.prefix_commits..].to_vec());
        let (prefix_sets, suffix_sets) = self.sets.split_at(self.prefix_sets);
        let start = ProjectionCursor::start(initial_views(&self.sets, &self.current));
        let prefix = project_graph_after(
            &start,
            &LabelledTree::default(),
            &prefix_graph,
            prefix_sets,
            &self.reader,
        );
        let cursor = start.advanced(&prefix, &prefix_graph, prefix_sets);
        let mut tree = LabelledTree::default();
        tree.fold(&prefix_graph, prefix_sets);
        let suffix = project_graph_after(&cursor, &tree, &suffix_graph, suffix_sets, &self.reader);
        json!({
            "whole": projection_json(&project_graph(&whole, &self.sets, &self.current, &self.reader)),
            "prefix": projection_json(&prefix),
            "cursor": {
                "count": cursor.commit_count,
                "last": cursor.last_projected_id,
                "views": views_json(&cursor.views),
            },
            "tree": tree.files().iter().map(|(path, (blob, label))| json!({
                "path": path.as_str(),
                "blob": blob_key(blob),
                "label": label.as_str(),
            })).collect::<Vec<_>>(),
            "suffix": projection_json(&suffix),
            "preview": self.preview(false),
            "preview_with_counts": self.preview(true),
        })
    }

    fn preview(&self, include_private_counts: bool) -> Value {
        let graph = graph(self.commits.clone());
        let source = ProjectionPreviewSource {
            repo_id: &graph.repo_id,
            policy: &self.policy,
            graph: &graph,
            visibility_change_sets: &self.sets,
        };
        match projection_preview(
            source,
            &self.current,
            &self.reader,
            include_private_counts,
            &self.native,
        ) {
            Ok(preview) => json!({ "ok": preview_json(&preview) }),
            Err(error) => json!({ "error": error.to_string() }),
        }
    }
}

fn preview_json(preview: &ProjectionPreviewView) -> Value {
    json!({
        "repo_id": preview.repo_id,
        "view": preview.view.as_str(),
        "files": preview.files.iter().map(|file| json!({
            "path": file.path.as_str(),
            "oid": file.oid,
            "label": file.label.as_str(),
        })).collect::<Vec<_>>(),
        "commits": preview.commits.iter().map(|commit| json!({
            "projected_id": commit.projected_id,
            "logical_commit_id": commit.logical_commit_id,
            "parents": commit.parent_projected_ids,
            "author": commit.author,
            "message": commit.message,
            "visibility": match commit.visibility {
                ProjectionPreviewCommitVisibility::AllInView => "all",
                ProjectionPreviewCommitVisibility::SomeInView => "some",
                ProjectionPreviewCommitVisibility::NoneInView => "none",
            },
            "change_count": commit.change_count,
        })).collect::<Vec<_>>(),
        "summary": {
            "visible_files": preview.summary.visible_files,
            "hidden_files": preview.summary.hidden_files,
            "visible_commits": preview.summary.visible_commits,
            "hidden_commits": preview.summary.hidden_commits,
        },
    })
}

fn policy(rng: &mut Rng) -> Policy {
    let mut policy = Policy::new(label(rng));
    let rules = (0..rng.below(4))
        .map(|_| LabelRule {
            path: path(rng.pick(&PATHS)),
            view: label(rng),
        })
        .collect::<Vec<_>>();
    let _ = policy.add_rules(rules);
    policy
}

fn native_details(
    rng: &mut Rng,
    commits: &[LogicalCommit],
) -> BTreeMap<String, NativeRequestCommitDetails> {
    commits
        .iter()
        .flat_map(|commit| match &commit.origin {
            LogicalCommitOrigin::RequestMerge { commits, .. } => commits.clone(),
            _ => Vec::new(),
        })
        .filter_map(|native| {
            (rng.below(3) != 0).then(|| {
                (
                    native.oid.clone(),
                    NativeRequestCommitDetails {
                        author: format!("native-author{}", rng.below(2)),
                        message: format!("native {}", native.oid),
                        occurred_at_unix: 0,
                        changes: (0..rng.below(3))
                            .map(|_| FileChange {
                                path: path("/README.md"),
                                old_content: None,
                                new_content: None,
                                label: ViewId::public(),
                            })
                            .collect(),
                    },
                )
            })
        })
        .collect()
}

fn case(rng: &mut Rng) -> Case {
    let first = views(rng);
    let reader = id(rng.pick(&["public", "private", "c0", "c1"]));
    let label_ids = LABELS.map(id);
    let commits = history(rng, &label_ids);
    let (sets, current) = change_sets(rng, &commits, first);
    let policy = policy(rng);
    let native = native_details(rng, &commits);
    Case {
        policy,
        native,
        prefix_commits: rng.below(commits.len() + 1),
        prefix_sets: rng.below(sets.len() + 1),
        current,
        reader,
        commits,
        sets,
    }
}

fn projection_json(projection: &Projection) -> Value {
    json!({
        "repo_id": projection.repo_id,
        "view_key": projection.view_key.as_str(),
        "commits": projection
            .commits
            .iter()
            .map(|commit| json!({
                "projected_id": commit.projected_id,
                "logical_commit_id": commit.logical_commit_id,
                "parent": commit.parent_projected_id,
                "author": commit.author,
                "message": commit.message,
                "visibility_change_set_id": commit.visibility_change_set_id,
                "changes": commit.changes.iter().map(|change| json!({
                    "path": change.path.as_str(),
                    "label": change.label.as_str(),
                    "content": change.new_content.as_ref().map(blob_key),
                })).collect::<Vec<_>>(),
                "materialization": match &commit.materialization {
                    ProjectionMaterialization::Generate => json!({ "kind": "generate" }),
                    ProjectionMaterialization::PreserveGitCommit { oid, parent_oids, tree_oid } => json!({
                        "kind": "preserve",
                        "oid": oid,
                        "parents": parent_oids,
                        "tree": tree_oid,
                    }),
                },
            }))
            .collect::<Vec<_>>()
    })
}

pub(super) fn lean_outputs(cases: &[Value]) -> Vec<Value> {
    let model = std::env::var("SCOPE_PROJECTION_MODEL").expect(
        "SCOPE_PROJECTION_MODEL names the built Lean model; run dev/checks/projection-model",
    );
    let mut child = Command::new(model)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(Value::Array(cases.to_vec()).to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "the Lean model rejected the cases");
    serde_json::from_slice::<Vec<Value>>(&output.stdout).unwrap()
}

fn count(outputs: &[Value], kind: impl Fn(&Value) -> bool) -> usize {
    outputs
        .iter()
        .flat_map(|output| ["whole", "prefix", "suffix"].map(|part| &output[part]))
        .flat_map(|projection| projection["commits"].as_array().unwrap())
        .filter(|commit| kind(commit))
        .count()
}

fn previews(outputs: &[Value], kind: impl Fn(&Value) -> bool) -> usize {
    outputs
        .iter()
        .filter(|output| kind(&output["preview_with_counts"]))
        .count()
}

fn preview_commits(outputs: &[Value], visibility: &str) -> usize {
    outputs
        .iter()
        .filter_map(|output| output["preview"]["ok"]["commits"].as_array())
        .flatten()
        .filter(|commit| commit["visibility"] == visibility)
        .count()
}

#[test]
#[ignore = "needs the Lean model; run dev/checks/projection-model"]
fn the_rust_projection_matches_the_proven_lean_model() {
    let mut rng = Rng(0x5eed_5c09e);
    let cases = (0..CASES).map(|_| case(&mut rng)).collect::<Vec<_>>();
    let expected = cases.iter().map(Case::rust).collect::<Vec<_>>();
    let modelled = lean_outputs(&cases.iter().map(Case::json).collect::<Vec<_>>());
    assert_eq!(modelled.len(), cases.len());
    for ((case, rust), lean) in cases.iter().zip(&expected).zip(&modelled) {
        assert_eq!(
            rust,
            lean,
            "Rust and the Lean model disagree on {}",
            case.json()
        );
    }
    let from_set = |commit: &Value| !commit["visibility_change_set_id"].is_null();
    let message = |commit: &Value, text: &str| commit["message"] == text;
    let anchored = cases
        .iter()
        .flat_map(|case| case.sets.iter().map(move |set| (case, set)))
        .filter(|(case, set)| {
            set.anchor_commit_id
                .as_ref()
                .is_some_and(|anchor| case.commits.iter().any(|commit| &commit.id == anchor))
        })
        .count();
    let relabels_of_source_paths = cases
        .iter()
        .flat_map(|case| case.sets.iter().map(move |set| (case, set)))
        .map(|(case, set)| {
            set.changes
                .iter()
                .filter(|change| {
                    case.commits.iter().any(|commit| {
                        set.source_update_id.as_ref() == Some(&commit.id)
                            && commit
                                .changes
                                .iter()
                                .any(|source| source.path == change.path)
                    })
                })
                .count()
        })
        .sum::<usize>();
    let resumed_after_a_transition = cases
        .iter()
        .zip(&expected)
        .filter(|(case, output)| {
            case.sets[..case.prefix_sets]
                .iter()
                .any(|set| set.views.is_some())
                && !output["suffix"]["commits"].as_array().unwrap().is_empty()
        })
        .count();
    let reached = [
        (
            "partial commits",
            count(&expected, |c| {
                !from_set(c) && message(c, "Projected public update")
            }),
        ),
        (
            "preserved commits",
            count(&expected, |c| c["materialization"]["kind"] == "preserve"),
        ),
        (
            "source updates",
            count(&expected, |c| {
                from_set(c) && message(c, "Projected public update")
            }),
        ),
        (
            "view boundaries",
            count(&expected, |c| message(c, "Projection view boundary")),
        ),
        (
            "baselines",
            count(&expected, |c| message(c, "Projection baseline")),
        ),
        (
            "visibility boundaries",
            count(&expected, |c| message(c, "Projection visibility boundary")),
        ),
        ("anchored sets", anchored),
        ("commits partly in view", preview_commits(&expected, "some")),
        ("commits not in view", preview_commits(&expected, "none")),
        (
            "hidden file counts",
            previews(&expected, |preview| {
                preview["ok"]["summary"]["hidden_files"].as_u64() > Some(0)
            }),
        ),
        (
            "hidden commit counts",
            previews(&expected, |preview| {
                preview["ok"]["summary"]["hidden_commits"].as_u64() > Some(0)
            }),
        ),
        (
            "relabels of a source update's paths",
            relabels_of_source_paths,
        ),
        (
            "resumed folds after a transition",
            resumed_after_a_transition,
        ),
    ];
    let missing_details = previews(&expected, |preview| preview.get("error").is_some());
    assert!(
        missing_details > 10,
        "the generator stopped reaching missing native details ({missing_details})"
    );
    for (kind, reached) in reached {
        assert!(
            reached > 50,
            "the generator stopped reaching {kind} ({reached})"
        );
    }
}
