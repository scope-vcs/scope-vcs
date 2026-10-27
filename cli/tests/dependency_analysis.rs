mod support;
use scope_cli::{local_dependency_analysis::analyze_snapshot_with, progress::CancellationToken};
use scope_domain::dependency_analysis::{AnalyzerOutput, DependencyEdge, DependencyEdgeKind};
use std::{fs, path::Path};
use support::TempDir;

fn analyze(files: &[(&str, &str)]) -> anyhow::Result<AnalyzerOutput> {
    let snapshot = TempDir::new("dependency-analysis");
    for (path, contents) in files {
        let path = snapshot.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
    analyze_snapshot_with(
        Path::new(env!("CARGO_BIN_EXE_scope")),
        snapshot.path(),
        &CancellationToken::new(),
    )
}

#[test]
fn analyzer_child_reports_snapshot_imports() {
    let output = analyze(&[
        ("src/index.ts", "import '../internal/client';\n"),
        ("internal/client.ts", "export const secret = 1;\n"),
    ])
    .unwrap();

    assert_eq!(
        output.edges,
        [DependencyEdge {
            source_path: "src/index.ts".into(),
            target_path: "internal/client.ts".into(),
            kind: DependencyEdgeKind::SideEffectImport,
        }]
    );
    assert!(output.gaps.is_empty(), "{:?}", output.gaps);
}

#[test]
fn deeply_nested_source_fails_the_analysis_without_aborting_the_cli() {
    let depth = 200_000;
    let nested = format!(
        "export const value = {}1{};\n",
        "(".repeat(depth),
        ")".repeat(depth)
    );

    let error = analyze(&[("src/nested.ts", &nested)]).unwrap_err();

    assert!(
        format!("{error:#}").contains("Dependency analyzer failed"),
        "{error:#}"
    );
}
