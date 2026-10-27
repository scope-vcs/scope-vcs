use super::*;
use scope_domain::dependency_analysis::{DependencyEdge, DependencyEdgeKind};

fn output_with_reason(reason: String) -> AnalyzerOutput {
    AnalyzerOutput {
        gaps: vec![DependencyGap {
            path: "src/main.ts".into(),
            reason,
        }],
        ..incomplete_base()
    }
}

#[test]
fn oversized_results_become_a_bounded_coverage_gap() {
    let output = AnalyzerOutput {
        analyzed_files: (0..2_200)
            .map(|index| format!("{}{index}.ts", "a/".repeat(2_000)))
            .collect(),
        ..incomplete_base()
    };
    assert!(serialized_len(&output) > MAX_OUTPUT_BYTES);

    let bounded = within_output_budget(output);

    assert!(serialized_len(&bounded) <= MAX_OUTPUT_BYTES);
    assert_eq!(bounded.analyzer_version, DEPENDENCY_ANALYZER_VERSION);
    assert!(bounded.analyzed_files.is_empty());
    assert_eq!(bounded.gaps.len(), 1);
    assert_eq!(bounded.gaps[0].path, ".");
    assert!(bounded.gaps[0].reason.contains("analysis is incomplete"));
}

#[test]
fn output_budget_counts_utf8_bytes() {
    let overhead = serialized_len(&output_with_reason(String::new()));
    let fitting = output_with_reason("é".repeat((MAX_OUTPUT_BYTES - overhead) / 2));
    assert_eq!(within_output_budget(fitting.clone()), fitting);

    let mut oversized = fitting;
    oversized.gaps[0].reason.push('é');
    assert_eq!(within_output_budget(oversized).gaps[0].path, ".");
}

#[cfg(target_os = "linux")]
#[test]
fn analysis_over_its_memory_limit_fails_in_the_child_process() {
    const CHILD_SNAPSHOT: &str = "SCOPE_ANALYZER_MEMORY_TEST_SNAPSHOT";
    const SMALL_LIMIT_BYTES: u64 = 64 * 1024 * 1024;
    if let Some(snapshot) = std::env::var_os(CHILD_SNAPSHOT) {
        let arguments = [OsString::from("scope"), ANALYZE_COMMAND.into(), snapshot];
        let code = run_analysis_command(arguments.into_iter(), SMALL_LIMIT_BYTES);
        std::process::exit(if code == Some(ExitCode::SUCCESS) {
            0
        } else {
            2
        });
    }

    let snapshot = tempfile::TempDir::new().unwrap();
    let source = (0..24_000)
        .map(|index| format!("export const value{index} = {{ items: [1, 2, 3], scale: (x: number) => x * {index} }};\n"))
        .collect::<String>();
    for index in 0..4 {
        std::fs::write(
            snapshot.path().join(format!("generated{index}.ts")),
            &source,
        )
        .unwrap();
    }
    assert_eq!(analyze_snapshot(snapshot.path()).analyzed_files.len(), 4);

    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::analysis_over_its_memory_limit_fails_in_the_child_process",
            "--test-threads=1",
        ])
        .env(CHILD_SNAPSHOT, snapshot.path())
        .output()
        .unwrap();

    assert!(!child.status.success(), "{child:?}");
    assert!(
        String::from_utf8_lossy(&child.stderr).contains("memory allocation"),
        "{}",
        String::from_utf8_lossy(&child.stderr)
    );
}

#[test]
fn bounded_findings_match_collecting_everything_then_truncating() {
    let gaps = (0..limits::MAX_GAPS + 25)
        .rev()
        .map(|index| DependencyGap {
            path: format!("src/{:05}.ts", index % (limits::MAX_GAPS / 2)),
            reason: format!("unresolved import: ./{index}"),
        })
        .collect::<Vec<_>>();
    let mut findings = Findings::default();
    for index in 0..=limits::MAX_EDGES {
        findings.add_edge(DependencyEdge {
            source_path: format!("src/{index}.ts"),
            target_path: "src/target.ts".into(),
            kind: DependencyEdgeKind::Import,
        });
    }
    gaps.iter().cloned().for_each(|gap| findings.add_gap(gap));

    let mut expected = gaps;
    expected.push(repository_gap(format!(
        "dependency edge limit exceeded ({})",
        limits::MAX_EDGES
    )));
    expected.sort();
    expected.dedup();
    expected.truncate(limits::MAX_GAPS - 1);
    expected.push(repository_gap(format!(
        "coverage gap limit exceeded ({})",
        limits::MAX_GAPS
    )));
    let (edges, gaps) = findings.into_parts();
    assert_eq!(edges, []);
    assert_eq!(gaps, expected);
}
