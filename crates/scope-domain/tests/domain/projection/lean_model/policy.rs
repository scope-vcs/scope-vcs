use super::{Rng, id, lean_outputs, path};
use scope_domain::{
    policy::{LabelRule, Policy, PolicyError, ScopePath},
    views::Views,
};
use serde_json::{Value, json};

const CASES: usize = 3000;
const PATHS: [&str; 18] = [
    "/",
    "/a",
    "/a-b",
    "/a-b/c",
    "/a.b",
    "/a/a",
    "/a/a/a",
    "/a/child",
    "/ab",
    "/a/b",
    "/ab/c",
    "/a/b/c",
    "/a/bc",
    "/b",
    "/.scope",
    "/.scope/x",
    "/.scope/RULES.md",
    "/.scopex",
];
const LABELS: [&str; 3] = ["public", "private", "agent"];

fn rules(rng: &mut Rng, most: usize) -> Vec<LabelRule> {
    (0..rng.below(most + 1))
        .map(|_| LabelRule {
            path: path(rng.pick(&PATHS)),
            view: id(rng.pick(&LABELS)),
        })
        .collect()
}

fn labels(policy: &Policy, paths: &[ScopePath], views: &Views) -> Value {
    json!(
        paths
            .iter()
            .map(|path| policy.label(path, views).as_str().to_string())
            .collect::<Vec<_>>()
    )
}

#[test]
#[ignore = "needs the Lean model; run dev/checks/projection-model"]
fn policy_labels_and_the_public_island_rule_match_the_proven_policy() {
    let mut rng = Rng(0x0090_11c7);
    let views = Views::builtin();
    let paths = PATHS.map(path);
    let mut rust = Vec::new();
    let mut cases = Vec::new();
    let rule = |at: &str, view: &str| LabelRule {
        path: path(at),
        view: id(view),
    };
    let scenarios = [
        (
            "public",
            vec![rule("/a", "private")],
            vec![rule("/a-b", "public"), rule("/a/child", "public")],
        ),
        (
            "public",
            vec![rule("/a", "private")],
            vec![rule("/a-b", "private"), rule("/a/child", "public")],
        ),
        (
            "public",
            vec![rule("/", "public"), rule("/a", "private")],
            vec![rule("/a/a/a", "agent")],
        ),
    ]
    .map(|(default, existing, additions)| (id(default), existing, additions));
    let generated = (0..CASES).map(|_| {
        (
            id(rng.pick(&LABELS)),
            rules(&mut rng, 3),
            rules(&mut rng, 2),
        )
    });
    for (default, existing, additions) in scenarios.into_iter().chain(generated.collect::<Vec<_>>())
    {
        let mut policy = Policy::new(default);
        let _ = policy.add_rules(existing);
        let before = labels(&policy, &paths, &views);
        cases.push(json!({
            "kind": "policy",
            "policy": policy,
            "additions": additions,
            "full": views.full().as_str(),
            "paths": PATHS,
        }));
        let outcome = match policy.add_rules(additions) {
            Ok(()) => json!({ "ok": null }),
            Err(PolicyError::PublicIsland { child, parent }) => {
                json!({ "island": [child.as_str(), parent.as_str()] })
            }
        };
        rust.push(json!({
            "before": before,
            "outcome": outcome,
            "after": policy,
            "after_labels": labels(&policy, &paths, &views),
        }));
    }
    let modelled = lean_outputs(&cases);
    assert_eq!(modelled.len(), cases.len());
    for ((case, rust), lean) in cases.iter().zip(&rust).zip(&modelled) {
        assert_eq!(rust, lean, "Rust and the Lean policy disagree on {case}");
    }
    let islands = rust
        .iter()
        .filter(|output| output["outcome"].get("island").is_some())
        .count();
    let inherited = rust
        .iter()
        .filter(|output| {
            let rules = output["after"]["rules"].as_array().unwrap();
            let rule_at = |path: &str| rules.iter().any(|rule| rule["path"] == path);
            !rule_at("/a/b/c") && ["/", "/a", "/a/b"].iter().any(|path| rule_at(path))
        })
        .count();
    for (kind, reached) in [
        ("rejected islands", islands),
        ("labels inherited from an ancestor rule", inherited),
    ] {
        assert!(
            reached > 50,
            "the generator stopped reaching {kind} ({reached})"
        );
    }
}
