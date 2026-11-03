use super::*;

const WORKFLOW: &str = r#"
name: Test
on:
  manual: true
  push:
    branches:
      - main
container:
  image: rust:1.90@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 20m
caches:
  - name: cargo-target
    path: /scope/cache/cargo-target
    format: v1
    compatibility: { files: [], environment: [], source: false }
    exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false }
  - name: cargo
    path: /scope/cache/cargo
    format: v1
    compatibility: { files: [], environment: [], source: false }
    exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false }
env:
  RUSTUP_TOOLCHAIN: stable
jobs:
  checks:
    env:
      TEST_MODE: strict
    steps:
      - name: Format
        run: cargo fmt --check
      - name: Test
        run: cargo test --workspace
"#;

#[test]
fn parses_and_normalizes_jobs_workflow() {
    let parsed = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes()).unwrap();
    let definition = parsed.definition();

    assert_eq!(parsed.path().name(), "test");
    assert_eq!(definition.name(), "Test");
    assert!(definition.triggers().manual());
    assert!(definition.triggers().push_main());
    assert!(!definition.triggers().request());
    let on_request = WORKFLOW.replace("manual: true", "manual: true\n  request: true");
    let parsed = parse_workflow("/.scope/runs/test.yml", on_request.as_bytes()).unwrap();
    assert!(parsed.definition().triggers().request());
    let job = &definition.jobs()[0];
    assert_eq!(job.id().as_str(), "checks");
    assert_eq!(job.timeout_seconds(), 20 * 60);
    assert_eq!(
        job.container().image(),
        "rust:1.90@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(job.environment()["RUSTUP_TOOLCHAIN"], "stable");
    assert_eq!(job.environment()["TEST_MODE"], "strict");
    assert_eq!(
        job.caches()
            .iter()
            .map(WorkflowCache::as_str)
            .collect::<Vec<_>>(),
        ["cargo", "cargo-target"]
    );
    assert_eq!(job.steps()[1].run(), "cargo test --workspace");
}

#[test]
fn equivalent_yaml_has_the_same_revision_digest() {
    let compact = r#"
name: Test
on: { push: true, manual: true }
container: { image: "rust:1.90@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" }
timeout: 1200s
caches:
  - { name: cargo, path: /scope/cache/cargo, format: v1, compatibility: { files: [], environment: [], source: false }, exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false } }
  - { name: cargo-target, path: /scope/cache/cargo-target, format: v1, compatibility: { files: [], environment: [], source: false }, exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false } }
env: { TEST_MODE: strict, RUSTUP_TOOLCHAIN: stable }
jobs:
  checks:
    steps:
      - { name: Format, run: "cargo fmt --check" }
      - { name: Test, run: "cargo test --workspace" }
"#;
    let first = parse_workflow("/.scope/runs/test.yml", WORKFLOW.as_bytes())
        .unwrap()
        .into_revision("repo-1")
        .unwrap();
    let second = parse_workflow("/.scope/runs/test.yaml", compact.as_bytes())
        .unwrap()
        .into_revision("repo-1")
        .unwrap();

    assert_eq!(first.digest(), second.digest());
}

#[test]
fn rejects_unknown_keys_and_non_main_pushes() {
    let unknown = WORKFLOW.replace("name: Test", "name: Test\nmatrix: {}");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", unknown.as_bytes()),
        Err(RunConfigError::InvalidYaml(_))
    ));

    let branch = WORKFLOW.replace("- main", "- feature");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", branch.as_bytes()),
        Err(RunConfigError::UnsupportedPushBranches)
    ));

    for invalid in ["1INVALID", "WITH-DASH"] {
        let workflow = WORKFLOW.replace("RUSTUP_TOOLCHAIN:", &format!("{invalid}:"));
        assert!(matches!(
            parse_workflow("/.scope/runs/test.yml", workflow.as_bytes()),
            Err(RunConfigError::InvalidWorkflow(
                WorkflowError::InvalidEnvironmentKey
            ))
        ));
    }
}

#[test]
fn rejects_missing_triggers_invalid_timeout_and_oversized_definitions() {
    let no_triggers = WORKFLOW
        .replace("manual: true", "manual: false")
        .replace("push:\n    branches:\n      - main", "push: false");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", no_triggers.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::MissingTrigger
        ))
    ));

    let invalid_timeout = WORKFLOW.replace("timeout: 20m", "timeout: soon");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", invalid_timeout.as_bytes()),
        Err(RunConfigError::InvalidTimeout)
    ));

    let oversized = vec![b'a'; MAX_WORKFLOW_DEFINITION_BYTES + 1];
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", &oversized),
        Err(RunConfigError::DefinitionTooLarge)
    ));
}

#[test]
fn caches_default_to_empty_and_reject_invalid_or_ambiguous_mounts() {
    let missing = WORKFLOW.replace(
            "caches:\n  - name: cargo-target\n    path: /scope/cache/cargo-target\n    format: v1\n    compatibility: { files: [], environment: [], source: false }\n    exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false }\n  - name: cargo\n    path: /scope/cache/cargo\n    format: v1\n    compatibility: { files: [], environment: [], source: false }\n    exact: { files: [Cargo.lock], environment: [RUSTUP_TOOLCHAIN], source: false }\n",
            "",
        );
    assert!(
        parse_workflow("/.scope/runs/test.yml", missing.as_bytes())
            .unwrap()
            .definition()
            .jobs()[0]
            .caches()
            .is_empty()
    );

    let invalid = WORKFLOW.replace("name: cargo-target", "name: Cargo_Target");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", invalid.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::InvalidCache(_)
        ))
    ));

    let duplicate = WORKFLOW.replace("name: cargo\n", "name: cargo-target\n");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", duplicate.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::DuplicateCacheName(name)
        )) if name == "cargo-target"
    ));

    let overlapping = WORKFLOW.replace(
        "    path: /scope/cache/cargo\n",
        "    path: /scope/cache/cargo-target/registry\n",
    );
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", overlapping.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::OverlappingCachePath(_)
        ))
    ));
}

#[test]
fn jobs_inherit_and_can_override_workflow_runtime_defaults() {
    let workflow = r#"
name: Graph
on: { manual: true }
container: { image: rust:1.90@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa }
timeout: 20m
caches: [{ name: cargo, path: /scope/cache/cargo, format: v1, compatibility: { files: [], environment: [], source: false }, exact: { files: [], environment: [], source: false } }]
env: { SHARED: workflow, WORKFLOW_ONLY: yes }
jobs:
  backend:
    env: { SHARED: backend }
    steps:
      - { name: Backend, run: cargo test }
  web:
    needs: [backend]
    container: { image: node:24@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb }
    timeout: 5m
    caches: []
    steps:
      - { name: Web, run: pnpm test }
"#;
    let parsed = parse_workflow("/.scope/runs/graph.yml", workflow.as_bytes()).unwrap();
    let definition = parsed.definition();
    let backend = definition
        .job(&WorkflowJobId::parse("backend").unwrap())
        .unwrap();
    assert_eq!(
        backend.container().image(),
        "rust:1.90@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    );
    assert_eq!(backend.timeout_seconds(), 20 * 60);
    assert_eq!(backend.caches()[0].as_str(), "cargo");
    assert_eq!(backend.environment()["SHARED"], "backend");
    assert_eq!(backend.environment()["WORKFLOW_ONLY"], "yes");

    let web = definition
        .job(&WorkflowJobId::parse("web").unwrap())
        .unwrap();
    assert_eq!(web.needs()[0].as_str(), "backend");
    assert_eq!(
        web.container().image(),
        "node:24@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
    );
    assert_eq!(web.timeout_seconds(), 5 * 60);
    assert!(web.caches().is_empty());
    assert_eq!(web.environment()["SHARED"], "workflow");
}

#[test]
fn rejects_invalid_graphs() {
    let missing = WORKFLOW.replace("jobs:\n  checks:", "jobs:\n  checks:\n    needs: [missing]");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", missing.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::MissingDependency { .. }
        ))
    ));

    let duplicate_job =
        format!("{WORKFLOW}  checks:\n    steps:\n      - {{ name: Duplicate, run: 'true' }}\n");
    assert!(matches!(
        parse_workflow("/.scope/runs/test.yml", duplicate_job.as_bytes()),
        Err(RunConfigError::InvalidWorkflow(
            WorkflowError::DuplicateJobId(id)
        )) if id == "checks"
    ));
}

fn job_graph_workflow(needs: &[Vec<usize>]) -> String {
    let jobs = needs
        .iter()
        .enumerate()
        .map(|(job, needs)| {
            let needs = needs
                .iter()
                .map(|need| format!("j{need}"))
                .collect::<Vec<_>>();
            format!(
                "  j{job}:\n    needs: [{}]\n    steps:\n      - {{ name: Run, run: 'true' }}\n",
                needs.join(", ")
            )
        })
        .collect::<String>();
    let (head, _) = WORKFLOW.split_once("jobs:\n").unwrap();
    format!("{head}jobs:\n{jobs}")
}

fn acyclic(needs: &[Vec<usize>]) -> bool {
    let mut done = vec![false; needs.len()];
    let mut progressed = true;
    while progressed {
        progressed = false;
        for job in 0..needs.len() {
            if !done[job] && needs[job].iter().all(|need| done[*need]) {
                done[job] = true;
                progressed = true;
            }
        }
    }
    done.into_iter().all(|done| done)
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 128, ..Default::default() })]

    #[test]
    fn generated_job_graphs_are_accepted_exactly_when_they_are_valid(
        needs in proptest::collection::vec(proptest::collection::vec(0..5usize, 0..3), 1..5)
    ) {
        let known = needs.len();
        let parsed = parse_workflow("/.scope/runs/test.yml", job_graph_workflow(&needs).as_bytes());
        let missing = needs.iter().flatten().any(|need| *need >= known);
        let itself = needs.iter().enumerate().any(|(job, needs)| needs.contains(&job));
        let repeated = needs
            .iter()
            .any(|needs| needs.iter().collect::<std::collections::BTreeSet<_>>().len() != needs.len());
        let valid = !missing && !itself && !repeated && acyclic(&needs);
        proptest::prop_assert_eq!(parsed.is_ok(), valid, "{:?} {:?}", needs, parsed.err());
    }

    #[test]
    fn arbitrary_or_corrupted_workflows_are_rejected_without_panicking(
        bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..256),
        cut in 0..WORKFLOW.len(),
        splice in "[\\-:{}\\[\\]&*!|>'\"%@`#, \\n\\t]{0,8}",
        path in "/\\.scope/(runs|workflows)/[a-z./-]{0,12}",
    ) {
        let _ = parse_workflow(&path, &bytes);
        let corrupted = format!("{}{}{}", &WORKFLOW[..cut], splice, &WORKFLOW[cut..]);
        if let Ok(parsed) = parse_workflow(&path, corrupted.as_bytes()) {
            proptest::prop_assert_eq!(parsed.path().as_str(), path.as_str());
            proptest::prop_assert!(!path.contains("/../") && !path.ends_with("/.."));
        }
    }
}

#[test]
fn workflow_set_rejects_ambiguous_path_stems() {
    let error = parse_workflow_set(
        "repo-1",
        [
            ("/.scope/runs/test.yml", WORKFLOW.as_bytes()),
            ("/.scope/runs/test.yaml", WORKFLOW.as_bytes()),
        ],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        RunConfigError::DuplicateWorkflowName(name) if name == "test"
    ));
}

#[test]
fn repository_catalog_uses_the_existing_workflow_parser() {
    use scope_domain::{
        content::DEFAULT_GIT_FILE_MODE,
        runs::catalog::{RepositoryWorkflowCatalog, RepositoryWorkflowFile},
    };

    let file = RepositoryWorkflowFile::from_content(
        "/.scope/runs/test.yml",
        DEFAULT_GIT_FILE_MODE,
        WORKFLOW.as_bytes().to_vec(),
    )
    .unwrap();
    let catalog = RepositoryWorkflowCatalog::captured(
        "repo-1",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        3,
        vec![file],
    )
    .unwrap();

    let revisions = parse_repository_workflow_catalog(&catalog).unwrap();
    assert_eq!(revisions.len(), 1);
    assert_eq!(revisions[0].workflow().repository_id(), "repo-1");
    assert_eq!(
        revisions[0].workflow().path().as_str(),
        "/.scope/runs/test.yml"
    );
}

#[test]
fn repository_catalog_surfaces_capture_errors_without_parsing() {
    use scope_domain::runs::catalog::RepositoryWorkflowCatalog;

    let catalog = RepositoryWorkflowCatalog::rejected(
        "repo-1",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        3,
        "invalid workflow path /.scope/runs/Bad.yml",
    )
    .unwrap();

    assert!(matches!(
        parse_repository_workflow_catalog(&catalog),
        Err(RunConfigError::CatalogRejected(message))
            if message == "invalid workflow path /.scope/runs/Bad.yml"
    ));
}
