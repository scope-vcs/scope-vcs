use scope_domain::github_check_catalog::{
    GITHUB_JOB_CONDITION_MESSAGE, GITHUB_MAIN_PULL_REQUESTS_MESSAGE, GITHUB_PATH_FILTER_MESSAGE,
    GITHUB_STACKED_PULL_REQUESTS_MESSAGE, GITHUB_WORKFLOW_DISABLED_MESSAGE, GitHubCatalogCheck,
    GitHubCheckCatalog, GitHubCheckRuns, GitHubCheckSources, GitHubWorkflowFile,
    GitHubWorkflowState, GitHubWorkflowSummary, WorkflowValue, github_check_catalog,
};

fn s(value: &str) -> WorkflowValue {
    WorkflowValue::String(value.to_string())
}

fn seq(items: &[WorkflowValue]) -> WorkflowValue {
    WorkflowValue::Sequence(items.to_vec())
}

fn strs(items: &[&str]) -> WorkflowValue {
    WorkflowValue::Sequence(items.iter().map(|item| s(item)).collect())
}

fn map(entries: &[(&str, WorkflowValue)]) -> WorkflowValue {
    WorkflowValue::Mapping(
        entries
            .iter()
            .map(|(key, value)| ((*key).to_string(), value.clone()))
            .collect(),
    )
}

fn file(path: &str, document: WorkflowValue) -> GitHubWorkflowFile {
    GitHubWorkflowFile {
        path: path.to_string(),
        document: Ok(document),
    }
}

fn workflow(on: WorkflowValue, jobs: WorkflowValue) -> WorkflowValue {
    map(&[("on", on), ("jobs", jobs)])
}

fn step_job() -> WorkflowValue {
    map(&[("runs-on", s("ubuntu-latest"))])
}

fn scope_push() -> WorkflowValue {
    map(&[("push", map(&[("branches", strs(&["scope/**"]))]))])
}

fn sources(files: Vec<GitHubWorkflowFile>) -> GitHubCheckSources {
    GitHubCheckSources {
        workflow_files: files,
        workflow_states: Vec::new(),
        required_on_github: Vec::new(),
        reported_on_main: Vec::new(),
        opens_pull_requests: false,
    }
}

fn yes() -> GitHubCheckRuns {
    GitHubCheckRuns::Yes
}

fn maybe(reason: &str) -> GitHubCheckRuns {
    GitHubCheckRuns::Maybe(reason.to_string())
}

fn check(name: &str, path: &str, runs: GitHubCheckRuns) -> GitHubCatalogCheck {
    GitHubCatalogCheck {
        name: name.to_string(),
        workflow_path: Some(path.to_string()),
        runs: Some(runs),
        required_on_github: false,
        reported_on_main: false,
    }
}

fn job_names(
    job: WorkflowValue,
    extra_files: Vec<GitHubWorkflowFile>,
) -> (Vec<String>, Vec<String>) {
    let mut files = vec![file(
        ".github/workflows/ci.yml",
        workflow(scope_push(), map(&[("build", job)])),
    )];
    files.extend(extra_files);
    let catalog = github_check_catalog(sources(files));
    let ci = catalog
        .workflows
        .iter()
        .find(|summary| summary.path == ".github/workflows/ci.yml")
        .unwrap();
    (
        catalog
            .checks
            .iter()
            .filter(|check| check.workflow_path.as_deref() == Some(ci.path.as_str()))
            .map(|check| check.name.clone())
            .collect(),
        ci.runtime_named_jobs.clone(),
    )
}

#[test]
fn this_repository_runs_on_request_branches_and_flags_conditional_jobs() {
    let document = map(&[
        ("name", s("CI")),
        (
            "on",
            map(&[
                ("pull_request", WorkflowValue::Null),
                ("push", map(&[("branches", strs(&["scope/**"]))])),
            ]),
        ),
        (
            "jobs",
            map(&[
                ("plan", step_job()),
                (
                    "release",
                    map(&[
                        ("name", s("Release")),
                        ("if", s("github.ref == 'refs/heads/main'")),
                    ]),
                ),
            ]),
        ),
    ]);
    let catalog = github_check_catalog(sources(vec![file(".github/workflows/ci.yml", document)]));

    assert_eq!(
        catalog,
        GitHubCheckCatalog {
            workflows: vec![GitHubWorkflowSummary {
                path: ".github/workflows/ci.yml".to_string(),
                name: "CI".to_string(),
                runs: yes(),
                runtime_named_jobs: Vec::new(),
            }],
            checks: vec![
                check(
                    "Release",
                    ".github/workflows/ci.yml",
                    maybe(GITHUB_JOB_CONDITION_MESSAGE)
                ),
                check("plan", ".github/workflows/ci.yml", yes()),
            ],
        }
    );
}

#[test]
fn workflow_triggers_decide_whether_request_branches_run() {
    let push = |config: WorkflowValue| map(&[("push", config)]);
    let pull_request = |config: WorkflowValue| map(&[("pull_request", config)]);
    let cases = [
        ("bare push", s("push"), false, Some(yes())),
        (
            "event list",
            strs(&["pull_request", "push"]),
            false,
            Some(yes()),
        ),
        ("null push", push(WorkflowValue::Null), false, Some(yes())),
        (
            "single segment glob",
            push(map(&[("branches", strs(&["scope/*"]))])),
            false,
            None,
        ),
        (
            "branch string",
            push(map(&[("branches", s("scope/**"))])),
            false,
            Some(yes()),
        ),
        (
            "later negation excludes",
            push(map(&[(
                "branches",
                strs(&["scope/**", "!scope/requests/*"]),
            )])),
            false,
            None,
        ),
        (
            "ignored branches",
            push(map(&[("branches-ignore", strs(&["scope/**"]))])),
            false,
            None,
        ),
        (
            "other ignored branches",
            push(map(&[("branches-ignore", strs(&["main"]))])),
            false,
            Some(yes()),
        ),
        (
            "tags only",
            push(map(&[("tags", strs(&["v*"]))])),
            false,
            None,
        ),
        (
            "tags with branches",
            push(map(&[("tags", strs(&["v*"])), ("branches", strs(&["**"]))])),
            false,
            Some(yes()),
        ),
        (
            "push paths",
            push(map(&[
                ("branches", strs(&["scope/**"])),
                ("paths", strs(&["web/**"])),
            ])),
            false,
            Some(maybe(GITHUB_PATH_FILTER_MESSAGE)),
        ),
        (
            "pull request without Scope pull requests",
            pull_request(WorkflowValue::Null),
            false,
            None,
        ),
        (
            "pull request with Scope pull requests",
            pull_request(WorkflowValue::Null),
            true,
            Some(yes()),
        ),
        (
            "pull request into another base",
            pull_request(map(&[("branches", strs(&["release/**"]))])),
            true,
            None,
        ),
        (
            "pull request into main only",
            pull_request(map(&[("branches", strs(&["main"]))])),
            true,
            Some(maybe(GITHUB_MAIN_PULL_REQUESTS_MESSAGE)),
        ),
        (
            "pull request into request branches only",
            pull_request(map(&[("branches", strs(&["scope/**"]))])),
            true,
            Some(maybe(GITHUB_STACKED_PULL_REQUESTS_MESSAGE)),
        ),
        (
            "pull request into main and request branches",
            pull_request(map(&[("branches", strs(&["main", "scope/**"]))])),
            true,
            Some(yes()),
        ),
        (
            "pull request closed only",
            pull_request(map(&[("types", strs(&["closed", "labeled"]))])),
            true,
            None,
        ),
        (
            "pull request synchronize",
            pull_request(map(&[("types", strs(&["labeled", "synchronize"]))])),
            true,
            Some(yes()),
        ),
        (
            "pull request paths",
            pull_request(map(&[("paths-ignore", strs(&["docs/**"]))])),
            true,
            Some(maybe(GITHUB_PATH_FILTER_MESSAGE)),
        ),
        (
            "unfiltered event wins over path filter",
            map(&[
                ("push", map(&[("paths", strs(&["web/**"]))])),
                ("pull_request", WorkflowValue::Null),
            ]),
            true,
            Some(yes()),
        ),
        (
            "pull request target",
            map(&[("pull_request_target", WorkflowValue::Null)]),
            true,
            None,
        ),
        ("manual only", s("workflow_dispatch"), true, None),
    ];
    for (label, on, opens_pull_requests, expected) in cases {
        let mut sources = sources(vec![file(
            ".github/workflows/ci.yml",
            workflow(on, map(&[("build", step_job())])),
        )]);
        sources.opens_pull_requests = opens_pull_requests;
        let catalog = github_check_catalog(sources);
        let runs = &catalog.workflows[0].runs;
        match &expected {
            Some(expected) => assert_eq!(runs, expected, "{label}"),
            None => assert!(
                matches!(runs, GitHubCheckRuns::No(message) if message.contains("scope/**")),
                "{label}: {runs:?}"
            ),
        }
        assert_eq!(catalog.checks[0].runs.as_ref(), Some(runs), "{label}");
    }
}

#[test]
fn unusable_workflows_report_why_and_reusable_only_files_are_omitted() {
    let mut sources = sources(vec![
        GitHubWorkflowFile {
            path: ".github/workflows/broken.yml".to_string(),
            document: Err("mapping values are not allowed here".to_string()),
        },
        file(
            ".github/workflows/off.yml",
            workflow(scope_push(), map(&[("lint", step_job())])),
        ),
        file(
            ".github/workflows/shared.yml",
            workflow(
                map(&[("workflow_call", WorkflowValue::Null)]),
                map(&[("test", step_job())]),
            ),
        ),
    ]);
    sources.workflow_states = vec![GitHubWorkflowState {
        path: ".github/workflows/off.yml".to_string(),
        active: false,
    }];
    let catalog = github_check_catalog(sources);

    let summaries = catalog
        .workflows
        .iter()
        .map(|summary| (summary.path.as_str(), summary.name.as_str(), &summary.runs))
        .collect::<Vec<_>>();
    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].0, ".github/workflows/broken.yml");
    assert_eq!(summaries[0].1, "broken.yml");
    assert!(matches!(
        summaries[0].2,
        GitHubCheckRuns::No(message) if message.contains("mapping values are not allowed here")
    ));
    assert_eq!(
        summaries[1],
        (
            ".github/workflows/off.yml",
            "off.yml",
            &GitHubCheckRuns::No(GITHUB_WORKFLOW_DISABLED_MESSAGE.to_string())
        )
    );
    assert_eq!(
        catalog.checks,
        vec![check(
            "lint",
            ".github/workflows/off.yml",
            GitHubCheckRuns::No(GITHUB_WORKFLOW_DISABLED_MESSAGE.to_string())
        )]
    );
}

#[test]
fn job_check_names_expand_matrices_or_defer_to_runtime() {
    let matrix_job = |name: Option<&str>, matrix: WorkflowValue| {
        let mut entries = vec![("strategy", map(&[("matrix", matrix)]))];
        if let Some(name) = name {
            entries.push(("name", s(name)));
        }
        map(&entries)
    };
    let os_and_python = || {
        vec![
            ("os", strs(&["linux", "mac"])),
            (
                "python",
                seq(&[WorkflowValue::Real(3.1), WorkflowValue::Integer(3)]),
            ),
        ]
    };
    let cases: Vec<(&str, WorkflowValue, &[&str], &[&str])> = vec![
        ("plain", step_job(), &["build"], &[]),
        ("named", map(&[("name", s("Build"))]), &["Build"], &[]),
        (
            "matrix suffix",
            matrix_job(Some("Test"), map(&os_and_python())),
            &[
                "Test (linux, 3)",
                "Test (linux, 3.1)",
                "Test (mac, 3)",
                "Test (mac, 3.1)",
            ],
            &[],
        ),
        (
            "matrix exclude",
            matrix_job(None, {
                let mut entries = os_and_python();
                entries.push((
                    "exclude",
                    seq(&[map(&[
                        ("os", s("mac")),
                        ("python", WorkflowValue::Integer(3)),
                    ])]),
                ));
                map(&entries)
            }),
            &["build (linux, 3)", "build (linux, 3.1)", "build (mac, 3.1)"],
            &[],
        ),
        (
            "include only",
            matrix_job(
                None,
                map(&[(
                    "include",
                    seq(&[
                        map(&[("os", s("linux")), ("shard", WorkflowValue::Integer(1))]),
                        map(&[("os", s("mac")), ("debug", WorkflowValue::Bool(true))]),
                    ]),
                )]),
            ),
            &["build (linux, 1)", "build (mac, true)"],
            &[],
        ),
        (
            "matrix substituted name",
            matrix_job(
                Some("Test on ${{ matrix.os }}/${{matrix.python}}"),
                map(&os_and_python()),
            ),
            &[
                "Test on linux/3",
                "Test on linux/3.1",
                "Test on mac/3",
                "Test on mac/3.1",
            ],
            &[],
        ),
        (
            "include with dimensions",
            matrix_job(None, {
                let mut entries = os_and_python();
                entries.push(("include", seq(&[map(&[("os", s("windows"))])])));
                map(&entries)
            }),
            &[],
            &["build"],
        ),
        (
            "matrix from JSON",
            matrix_job(None, s("${{ fromJSON(needs.plan.outputs.matrix) }}")),
            &[],
            &["build"],
        ),
        (
            "non scalar matrix value",
            matrix_job(None, map(&[("os", seq(&[map(&[("name", s("linux"))])]))])),
            &[],
            &["build"],
        ),
        (
            "matrix larger than GitHub allows",
            matrix_job(None, {
                let values = (0..10).map(|value| value.to_string()).collect::<Vec<_>>();
                let values = values.iter().map(String::as_str).collect::<Vec<_>>();
                let axes = (0..10)
                    .map(|axis| format!("axis{axis}"))
                    .collect::<Vec<_>>();
                map(&axes
                    .iter()
                    .map(|axis| (axis.as_str(), strs(&values)))
                    .collect::<Vec<_>>())
            }),
            &[],
            &["build"],
        ),
        (
            "context in name",
            map(&[("name", s("Deploy ${{ github.ref_name }}"))]),
            &[],
            &["build"],
        ),
        (
            "unknown matrix key in name",
            matrix_job(Some("Test ${{ matrix.arch }}"), map(&os_and_python())),
            &[],
            &["build"],
        ),
    ];
    for (label, job, expected_names, expected_runtime) in cases {
        let (names, runtime) = job_names(job, Vec::new());
        assert_eq!(names, expected_names, "{label}");
        assert_eq!(runtime, expected_runtime, "{label}");
    }
}

#[test]
fn local_reusable_workflows_prefix_called_check_names() {
    let shared = |path: &str, jobs: WorkflowValue| {
        file(
            path,
            workflow(map(&[("workflow_call", WorkflowValue::Null)]), jobs),
        )
    };
    let calls = |target: &str| map(&[("uses", s(target))]);
    let unit = shared(
        ".github/workflows/unit.yml",
        map(&[
            ("test", map(&[("name", s("Unit"))])),
            ("coverage", map(&[("if", s("always()"))])),
        ]),
    );
    let nested = shared(
        ".github/workflows/nested.yml",
        map(&[("inner", calls("./.github/workflows/unit.yml"))]),
    );
    let ping = shared(
        ".github/workflows/ping.yml",
        map(&[("pong", calls("./.github/workflows/pong.yml"))]),
    );
    let pong = shared(
        ".github/workflows/pong.yml",
        map(&[("ping", calls("./.github/workflows/ping.yml"))]),
    );
    let cases: Vec<(&str, WorkflowValue, &[&str], &[&str])> = vec![
        (
            "local call",
            map(&[
                ("name", s("Suite")),
                ("uses", s("./.github/workflows/unit.yml")),
            ]),
            &["Suite / Unit", "Suite / coverage"],
            &[],
        ),
        (
            "nested call",
            calls("./.github/workflows/nested.yml"),
            &["build / inner / Unit", "build / inner / coverage"],
            &[],
        ),
        (
            "cycle",
            calls("./.github/workflows/ping.yml"),
            &[],
            &["build"],
        ),
        (
            "missing callee",
            calls("./.github/workflows/gone.yml"),
            &[],
            &["build"],
        ),
        (
            "remote callee",
            calls("octo/shared/.github/workflows/unit.yml@v1"),
            &[],
            &["build"],
        ),
        (
            "matrix caller",
            map(&[
                ("uses", s("./.github/workflows/unit.yml")),
                (
                    "strategy",
                    map(&[("matrix", map(&[("os", strs(&["linux"]))]))]),
                ),
            ]),
            &[],
            &["build"],
        ),
    ];
    for (label, job, expected_names, expected_runtime) in cases {
        let (names, runtime) = job_names(
            job,
            vec![unit.clone(), nested.clone(), ping.clone(), pong.clone()],
        );
        assert_eq!(names, expected_names, "{label}");
        assert_eq!(runtime, expected_runtime, "{label}");
    }

    let catalog = github_check_catalog(sources(vec![
        file(
            ".github/workflows/ci.yml",
            workflow(
                scope_push(),
                map(&[("suite", calls("./.github/workflows/unit.yml"))]),
            ),
        ),
        unit,
    ]));
    assert_eq!(
        catalog.checks,
        vec![
            check("suite / Unit", ".github/workflows/ci.yml", yes()),
            check(
                "suite / coverage",
                ".github/workflows/ci.yml",
                maybe(GITHUB_JOB_CONDITION_MESSAGE)
            ),
        ]
    );
}

#[test]
fn reusable_calls_that_fan_out_past_the_catalog_budget_are_named_at_runtime() {
    let job_ids = (0..100).map(|job| format!("j{job}")).collect::<Vec<_>>();
    let levels = (1..=4)
        .map(|level| {
            let job = if level == 4 {
                step_job()
            } else {
                map(&[(
                    "uses",
                    s(&format!("./.github/workflows/l{}.yml", level + 1)),
                )])
            };
            let jobs = job_ids
                .iter()
                .map(|id| (id.as_str(), job.clone()))
                .collect::<Vec<_>>();
            file(
                &format!(".github/workflows/l{level}.yml"),
                workflow(map(&[("workflow_call", WorkflowValue::Null)]), map(&jobs)),
            )
        })
        .collect::<Vec<_>>();
    let (names, runtime) = job_names(map(&[("uses", s("./.github/workflows/l1.yml"))]), levels);
    assert!(names.is_empty());
    assert_eq!(runtime, ["build"]);
}

#[test]
fn reusable_calls_stop_past_the_depth_limit() {
    let chain = (1..=6)
        .map(|level| {
            let jobs = if level == 6 {
                map(&[("leaf", step_job())])
            } else {
                map(&[(
                    "next",
                    map(&[(
                        "uses",
                        s(&format!("./.github/workflows/l{}.yml", level + 1)),
                    )]),
                )])
            };
            file(
                &format!(".github/workflows/l{level}.yml"),
                workflow(map(&[("workflow_call", WorkflowValue::Null)]), jobs),
            )
        })
        .collect::<Vec<_>>();
    let (shallow, shallow_runtime) = job_names(
        map(&[("uses", s("./.github/workflows/l3.yml"))]),
        chain.clone(),
    );
    assert_eq!(shallow, ["build / next / next / next / leaf"]);
    assert!(shallow_runtime.is_empty());

    let (deep, deep_runtime) = job_names(map(&[("uses", s("./.github/workflows/l1.yml"))]), chain);
    assert!(deep.is_empty());
    assert_eq!(deep_runtime, ["build"]);
}

#[test]
fn checks_merge_by_name_with_github_lists() {
    let mut sources = sources(vec![
        file(
            ".github/workflows/b.yml",
            workflow(scope_push(), map(&[("lint", step_job())])),
        ),
        file(
            ".github/workflows/a.yml",
            workflow(
                s("workflow_dispatch"),
                map(&[("lint", step_job()), ("Zeta", step_job())]),
            ),
        ),
        file(
            ".github/workflows/c.yml",
            workflow(
                map(&[("push", map(&[("paths", strs(&["web/**"]))]))]),
                map(&[("lint", step_job())]),
            ),
        ),
    ]);
    sources.required_on_github = vec!["lint".to_string(), "legacy".to_string()];
    sources.reported_on_main = vec!["Zeta".to_string(), "deploy".to_string()];
    let catalog = github_check_catalog(sources);

    assert_eq!(
        catalog
            .workflows
            .iter()
            .map(|summary| summary.path.as_str())
            .collect::<Vec<_>>(),
        [
            ".github/workflows/a.yml",
            ".github/workflows/b.yml",
            ".github/workflows/c.yml"
        ]
    );
    let listed_only = |name: &str, required: bool, reported: bool| GitHubCatalogCheck {
        name: name.to_string(),
        workflow_path: None,
        runs: None,
        required_on_github: required,
        reported_on_main: reported,
    };
    let no_trigger = catalog.workflows[0].runs.clone();
    assert!(matches!(no_trigger, GitHubCheckRuns::No(_)));
    assert_eq!(
        catalog.checks,
        vec![
            GitHubCatalogCheck {
                reported_on_main: true,
                ..check("Zeta", ".github/workflows/a.yml", no_trigger)
            },
            listed_only("deploy", false, true),
            listed_only("legacy", true, false),
            GitHubCatalogCheck {
                required_on_github: true,
                ..check("lint", ".github/workflows/b.yml", yes())
            },
        ]
    );
}
