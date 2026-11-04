use super::{id, lean_outputs, path, views_json};
use scope_domain::views::{ViewDefinition, ViewIncludes, ViewReaders, Views};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const CANDIDATES: [&str; 4] = ["public", "private", "c0", "c1"];
const READERS: [&str; 5] = ["public", "private", "c0", "c1", "unknown"];
const PATHS: [&str; 4] = [
    "/README.md",
    "/.scope",
    "/.scope/secret",
    "/.scope/RULES.md",
];

fn every_small_views() -> Vec<Views> {
    let includes = |mask: usize| -> BTreeSet<_> {
        (0..CANDIDATES.len())
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| id(CANDIDATES[bit]))
            .collect()
    };
    let mut all = Vec::new();
    for public in [true, false] {
        for public_mask in 0..16 {
            for customs in 0..=2u32 {
                for masks in 0..16usize.pow(customs) {
                    let mut definitions = Vec::<ViewDefinition>::from(Views::builtin());
                    if public {
                        definitions[0].includes = ViewIncludes::Some(includes(public_mask));
                    } else if public_mask == 0 {
                        definitions.remove(0);
                    } else {
                        continue;
                    }
                    for index in 0..customs as usize {
                        definitions.push(ViewDefinition {
                            id: id(&format!("c{index}")),
                            name: format!("C{index}"),
                            includes: ViewIncludes::Some(includes(masks >> (4 * index) & 15)),
                            readers: ViewReaders::Assigned,
                        });
                    }
                    all.extend(Views::new(definitions).ok());
                }
            }
        }
    }
    all
}

fn rust(views: &Views) -> Value {
    let readers = READERS.map(id);
    let paths = PATHS.map(path);
    json!({
        "valid": true,
        "anyone": views.readable_by(None).iter().map(|view| view.as_str()).collect::<Vec<_>>(),
        "readers": readers.iter().map(|reader| json!({
            "reader": reader.as_str(),
            "labels": views.labels(reader).iter().map(|label| label.as_str()).collect::<Vec<_>>(),
            "may_read": readers.iter().map(|target| views.may_read(reader, target)).collect::<Vec<_>>(),
            "readable_by": views.readable_by(Some(reader)).iter().map(|view| view.as_str()).collect::<Vec<_>>(),
            "shows": paths.iter().map(|path| readers.iter()
                .map(|label| views.shows(reader, path, label))
                .collect::<Vec<_>>()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "ids": views.iter().map(|definition| definition.id.as_str()).collect::<Vec<_>>(),
    })
}

#[test]
#[ignore = "needs the Lean model; run dev/checks/projection-model"]
fn every_small_views_configuration_matches_the_proven_view_rules() {
    let configurations = every_small_views();
    assert!(configurations.len() > 300);
    let cases = configurations
        .iter()
        .map(|views| {
            json!({
                "kind": "views",
                "views": views_json(views),
                "readers": READERS,
                "paths": PATHS,
            })
        })
        .collect::<Vec<_>>();
    let modelled = lean_outputs(&cases);
    assert_eq!(modelled.len(), cases.len());
    for ((views, case), lean) in configurations.iter().zip(&cases).zip(&modelled) {
        assert_eq!(
            &rust(views),
            lean,
            "Rust and the Lean view rules disagree on {case}"
        );
    }
}
