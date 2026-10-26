use super::*;

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
