use super::{blob, graph, path};
use scope_domain::{
    projection::{
        FileChange, LogicalCommit, LogicalCommitOrigin, NativeRequestCommit,
        ProjectionMaterialization, project_graph,
    },
    views::{ViewDefinition, ViewId, ViewIncludes, ViewReaders, Views},
};
use serde_json::{Value, json};
use std::{collections::BTreeSet, io::Write, process::Command};

const CASES: usize = 2000;
const PATHS: [&str; 7] = [
    "/README.md",
    "/src/a.rs",
    "/ops/b.txt",
    "/notes.txt",
    "/.scope",
    "/.scope/runs/x.yml",
    "/.scope/RULES.md",
];

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn chance(&mut self) -> bool {
        self.next() & 1 == 1
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

fn id(value: &str) -> ViewId {
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
            id: format!("c{index}"),
            origin: origin(rng, index, view_ids),
            author_id: format!("author{}", rng.below(2)),
            message: format!("message{}", rng.below(3)),
            changes: (0..rng.below(5))
                .map(|_| FileChange {
                    path: path(rng.pick(&PATHS)),
                    old_content: None,
                    new_content: rng.chance().then(|| blob(&format!("v{}", rng.below(3)))),
                    label: rng.pick(view_ids).clone(),
                })
                .collect(),
        })
        .collect()
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

fn case_json(views: &Views, reader: &ViewId, commits: &[LogicalCommit]) -> Value {
    json!({
        "views": views.iter().map(|definition| json!({
            "id": definition.id.as_str(),
            "includes": match &definition.includes {
                ViewIncludes::All => Value::Null,
                ViewIncludes::Some(included) => json!(included.iter().map(ViewId::as_str).collect::<Vec<_>>()),
            },
        })).collect::<Vec<_>>(),
        "reader": reader.as_str(),
        "commits": commits.iter().map(|commit| json!({
            "id": commit.id,
            "origin": origin_json(&commit.origin),
            "author": commit.author_id,
            "message": commit.message,
            "changes": commit.changes.iter().map(|change| json!({
                "path": change.path.as_str(),
                "label": change.label.as_str(),
                "content": change.new_content.as_ref().map(|blob| &blob.sha256),
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn rust_projection(views: &Views, reader: &ViewId, commits: Vec<LogicalCommit>) -> Value {
    let projection = project_graph(&graph(commits), &[], views, reader);
    json!(
        projection
            .commits
            .iter()
            .map(|commit| json!({
                "projected_id": commit.projected_id,
                "logical_commit_id": commit.logical_commit_id,
                "parent": commit.parent_projected_id,
                "author": commit.author,
                "message": commit.message,
                "changes": commit.changes.iter().map(|change| json!({
                    "path": change.path.as_str(),
                    "label": change.label.as_str(),
                    "content": change.new_content.as_ref().map(|blob| &blob.sha256),
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
    )
}

fn lean_projections(cases: &[Value]) -> Vec<Value> {
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

#[test]
#[ignore = "needs the Lean model; run dev/checks/projection-model"]
fn the_rust_projection_matches_the_proven_lean_model() {
    let mut rng = Rng(0x5eed_5c09e);
    let mut cases = Vec::new();
    let mut expected = Vec::new();
    for _ in 0..CASES {
        let views = views(&mut rng);
        let view_ids = views
            .iter()
            .map(|definition| definition.id.clone())
            .collect::<Vec<_>>();
        let reader = rng.pick(&view_ids).clone();
        let commits = history(&mut rng, &view_ids);
        cases.push(case_json(&views, &reader, &commits));
        expected.push(rust_projection(&views, &reader, commits));
    }
    let modelled = lean_projections(&cases);
    assert_eq!(modelled.len(), cases.len());
    for ((case, rust), lean) in cases.iter().zip(&expected).zip(&modelled) {
        assert_eq!(rust, lean, "Rust and the Lean model disagree on {case}");
    }
    let projected = expected
        .iter()
        .flat_map(|projection| projection.as_array().unwrap())
        .collect::<Vec<_>>();
    let partial = projected
        .iter()
        .filter(|commit| commit["message"] == "Projected public update")
        .count();
    let preserved = projected
        .iter()
        .filter(|commit| commit["projected_id"].as_str().unwrap().starts_with('n'))
        .count();
    assert!(
        partial > 500 && preserved > 50,
        "the generator stopped reaching partial ({partial}) or preserved ({preserved}) commits"
    );
}
