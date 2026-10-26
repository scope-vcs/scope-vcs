use scope_dependency_analyzer::analyze_snapshot;
use scope_domain::dependency_analysis::{
    AnalyzerOutput, DEPENDENCY_ANALYZER_VERSION, DependencyEdge, DependencyEdgeKind,
    DependencyEdgeKind::*, DependencyGap,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn analyze(root: &Path) -> AnalyzerOutput {
    analyze_snapshot(root)
}

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }
}

fn repository(files: &[(&str, &str)]) -> TempDir {
    let root = TempDir::new().unwrap();
    write_files(root.path(), files);
    root
}

fn edge(source_path: &str, target_path: &str, kind: DependencyEdgeKind) -> DependencyEdge {
    DependencyEdge {
        source_path: source_path.into(),
        target_path: target_path.into(),
        kind,
    }
}

fn gap(path: &str, reason: &str) -> DependencyGap {
    DependencyGap {
        path: path.into(),
        reason: reason.into(),
    }
}

fn targets_from(output: &AnalyzerOutput, source: &str) -> Vec<String> {
    output
        .edges
        .iter()
        .filter(|edge| edge.source_path == source)
        .map(|edge| edge.target_path.clone())
        .collect()
}

fn has_gap(output: &AnalyzerOutput, path: &str, reason: impl Fn(&str) -> bool) -> bool {
    output
        .gaps
        .iter()
        .any(|gap| gap.path == path && reason(&gap.reason))
}

#[test]
fn retains_static_import_forms_and_deduplicates_relative_and_aliased_targets() {
    let output = analyze(&fixture("static"));

    assert_eq!(output.analyzer_version, DEPENDENCY_ANALYZER_VERSION);
    assert_eq!(output.gaps, []);
    let pricing = "src/private/pricing.ts";
    assert_eq!(
        output.edges,
        [
            edge("src/public/alias.ts", pricing, Import),
            edge("src/public/direct.ts", pricing, Import),
            edge("src/public/duplicate.ts", pricing, Import),
            edge("src/public/literal-dynamic.ts", pricing, DynamicImport),
            edge("src/public/namespace.ts", pricing, Import),
            edge("src/public/reexport.ts", pricing, ReExport),
            edge("src/public/require.cjs", pricing, Require),
            edge(
                "src/public/side-effect.ts",
                "src/private/init.ts",
                SideEffectImport
            ),
            edge("src/public/type.ts", pricing, TypeImport),
        ]
    );
    assert!(
        output
            .analyzed_files
            .iter()
            .all(|path| !path.starts_with('/'))
    );
}

#[test]
fn keeps_known_edges_while_reporting_unresolved_nonliteral_and_syntax_gaps() {
    let output = analyze(&fixture("incomplete"));

    assert_eq!(
        output.edges,
        [edge("src/public/known.ts", "src/private/value.ts", Import)]
    );
    assert!(has_gap(&output, "src/public/missing.ts", |reason| reason
        .starts_with("unresolved import:")));
    let runtime = "src/public/runtime.ts";
    assert!(has_gap(&output, runtime, |reason| reason
        == "non-literal dynamic import"));
    assert!(has_gap(&output, runtime, |reason| reason == "non-literal require"));
    assert!(has_gap(&output, "src/public/syntax-error.ts", |reason| {
        reason.starts_with("syntax error:")
    }));
}

#[test]
fn reports_unsupported_languages_and_retains_internal_data_file_edges() {
    let output = analyze(&fixture("mixed"));

    assert_eq!(output.unsupported_files, ["src/main.rs", "src/private.py"]);
    assert_eq!(output.edges, [edge("src/config.ts", "data.json", Import)]);
    assert_eq!(output.gaps, []);
    assert_eq!(output.analyzed_files, ["src/config.ts"]);
}

#[test]
fn analyzes_uppercase_and_mixed_case_source_and_data_extensions() {
    let root = repository(&[
        ("src/APP.TS", "import './Util.Mjs';\n"),
        ("src/Util.Mjs", "import '../CONFIG.JSON';\n"),
        ("CONFIG.JSON", "{}"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.analyzed_files, ["src/APP.TS", "src/Util.Mjs"]);
    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [
            edge("src/APP.TS", "src/Util.Mjs", SideEffectImport),
            edge("src/Util.Mjs", "CONFIG.JSON", SideEffectImport),
        ]
    );
}

#[test]
fn resolves_aliases_inherited_from_a_parent_config() {
    for name in ["monorepo", "inherited-base-url"] {
        let output = analyze(&fixture(name));
        assert_eq!(output.gaps, [], "{name}");
        assert_eq!(
            output.edges,
            [edge(
                "packages/app/src/main.ts",
                "packages/shared/src/value.ts",
                Import
            )],
            "{name}"
        );
    }
}

#[test]
fn reports_invalid_config_without_losing_relative_edges() {
    let output = analyze(&fixture("bad-config"));

    assert_eq!(output.edges, [edge("src/entry.ts", "src/value.ts", Import)]);
    assert!(has_gap(&output, "tsconfig.json", |reason| reason
        .starts_with("invalid TypeScript config:")));
}

#[test]
fn analyzes_repository_tool_configuration_as_plain_source() {
    let output = analyze(&fixture("no-execution"));

    assert_eq!(output.gaps, []);
    assert_eq!(targets_from(&output, "src/entry.js"), ["src/value.js"]);
}

#[cfg(unix)]
#[test]
fn refuses_symbolic_links_instead_of_following_snapshot_escapes() {
    let outside = TempDir::new().unwrap();
    let root = repository(&[("src/main.ts", "export const safe = true;\n")]);
    std::os::unix::fs::symlink(
        outside.path().join("outside.ts"),
        root.path().join("src/escape.ts"),
    )
    .unwrap();

    let output = analyze(root.path());

    assert_eq!(output.analyzed_files, Vec::<String>::new());
    assert_eq!(output.edges, []);
    assert_eq!(
        output.gaps,
        [gap(".", "symbolic links are not analyzed: src/escape.ts")]
    );
}

#[test]
fn reports_imports_resolving_outside_the_snapshot_without_an_edge() {
    let parent = repository(&[
        ("outside.ts", "export const outside = true;\n"),
        (
            "repo/src/main.ts",
            "import { outside } from '../../outside';\nexport const escaped = outside;\n",
        ),
    ]);

    let output = analyze(&parent.path().join("repo"));

    assert_eq!(output.edges, []);
    assert_eq!(
        output.gaps,
        [gap(
            "src/main.ts",
            "resolved target escapes snapshot: ../../outside"
        )]
    );
}

#[test]
fn unresolved_bare_imports_are_external_unless_the_repository_claims_the_name() {
    let root = repository(&[
        ("package.json", r#"{"name":"@acme/app","private":true}"#),
        ("packages/shared/package.json", r#"{"name":"@acme/shared"}"#),
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@alias/*":["src/*"]}}}"#,
        ),
        ("src/private/secret.ts", "export const secret = 1;\n"),
        (
            "src/main.ts",
            "import React from 'react';\n\
             import { z } from 'zod/v4';\n\
             import path from 'node:path';\n\
             import { shared } from '@acme/shared';\n\
             import { missing } from '@alias/missing';\n\
             import { mapped } from '#internal/mapped';\n\
             import { secret } from './private/secret';\n\
             export const value = [React, z, path, shared, missing, mapped, secret];\n",
        ),
    ]);

    let output = analyze(root.path());

    assert_eq!(
        output.edges,
        [edge("src/main.ts", "src/private/secret.ts", Import)]
    );
    assert_eq!(
        output.gaps,
        [
            gap("src/main.ts", "unresolved import: #internal/mapped"),
            gap("src/main.ts", "unresolved import: @acme/shared"),
            gap("src/main.ts", "unresolved import: @alias/missing"),
        ]
    );
}

#[test]
fn inherited_paths_use_a_childs_explicit_base_url_as_typescript_does() {
    let root = repository(&[
        (
            "config/base.json",
            r#"{"compilerOptions":{"paths":{"@x/*":["src/*"]}}}"#,
        ),
        (
            "app/tsconfig.json",
            r#"{"extends":"../config/base.json","compilerOptions":{"baseUrl":"."}}"#,
        ),
        ("config/src/value.ts", "export const value = 1;\n"),
        ("app/src/value.ts", "export const value = 1;\n"),
        (
            "app/main.ts",
            "import { value } from '@x/value'; export { value };\n",
        ),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("app/main.ts", "app/src/value.ts", Import)]
    );
}

#[test]
fn resolves_package_self_exports_and_import_maps_without_installed_packages() {
    let root = repository(&[
        (
            "package.json",
            r##"{"name":"@acme/app","exports":{"./public":"./src/private/value.ts"},"imports":{"#internal":"./src/private/value.ts"}}"##,
        ),
        ("src/private/value.ts", "export const value = 1;\n"),
        (
            "src/main.ts",
            "import { value } from '@acme/app/public';\nexport { value as internal } from '#internal';\n",
        ),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [
            edge("src/main.ts", "src/private/value.ts", Import),
            edge("src/main.ts", "src/private/value.ts", ReExport),
        ]
    );
}

#[test]
fn rejects_extended_configs_outside_the_snapshot_and_keeps_relative_edges() {
    let parent = repository(&[
        (
            "outside.json",
            r#"{"compilerOptions":{"paths":{"@secret":["../outside.ts"]}}}"#,
        ),
        (
            "repo/tsconfig.json",
            "{ // JSONC is accepted\n \"extends\": \"../outside.json\",\n}\n",
        ),
        ("repo/src/main.ts", "import './value';\n"),
        ("repo/src/value.ts", "export const value = 1;\n"),
    ]);

    let output = analyze(&parent.path().join("repo"));

    assert_eq!(
        output.edges,
        [edge("src/main.ts", "src/value.ts", SideEffectImport)]
    );
    assert!(has_gap(&output, "tsconfig.json", |reason| reason
        .contains("escapes snapshot")));
}

#[test]
fn uses_import_require_and_types_package_export_conditions() {
    let conditions = "import { value } from '@acme/app/conditions';\n";
    let root = repository(&[
        (
            "package.json",
            r#"{"name":"@acme/app","exports":{"./conditions":{"types":"./src/private/types.d.ts","import":"./src/private/esm.ts","require":"./src/private/cjs.cts"}}}"#,
        ),
        ("src/private/types.d.ts", "export const value = 1;\n"),
        ("src/private/esm.ts", "export const value = 1;\n"),
        ("src/private/cjs.cts", "export const value = 1;\n"),
        ("src/import.ts", conditions),
        ("src/esm.mts", conditions),
        (
            "src/type.ts",
            "import type { value } from '@acme/app/conditions';\n",
        ),
        (
            "src/require.cts",
            "import value = require('@acme/app/conditions');\n",
        ),
        ("src/regular.cts", conditions),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [
            edge("src/esm.mts", "src/private/esm.ts", Import),
            edge("src/import.ts", "src/private/esm.ts", Import),
            edge("src/regular.cts", "src/private/cjs.cts", Import),
            edge("src/require.cts", "src/private/cjs.cts", Require),
            edge("src/type.ts", "src/private/types.d.ts", TypeImport),
        ]
    );
}

#[test]
fn rejects_path_aliases_that_point_outside_the_snapshot() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@outside/*":["../outside/*"]}}}"#,
        ),
        ("src/main.ts", "export const ok = true;\n"),
    ]);

    let output = analyze(root.path());

    assert!(has_gap(&output, "tsconfig.json", |reason| reason
        .contains("escapes its root")));
}

#[test]
fn ignores_installed_external_packages_in_the_snapshot_parent() {
    let parent = repository(&[
        (
            "node_modules/external/package.json",
            r#"{"main":"index.js"}"#,
        ),
        ("node_modules/external/index.js", "module.exports = 1;\n"),
        ("repo/src/main.ts", "import value from 'external';\n"),
    ]);

    let output = analyze(&parent.path().join("repo"));

    assert_eq!(output.edges, []);
    assert_eq!(output.gaps, []);
}

#[test]
fn bundler_query_imports_retain_the_underlying_source_edge() {
    let root = repository(&[
        (
            "src/main.ts",
            "export const worker = () => import('./worker.ts?worker');\n",
        ),
        ("src/worker.ts", "export const value = 1;\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("src/main.ts", "src/worker.ts", DynamicImport)]
    );
}

#[test]
fn root_dirs_overlay_relative_imports_without_discarding_inherited_aliases() {
    let root = repository(&[
        (
            "config/base.json",
            r#"{"compilerOptions":{"baseUrl":"..","paths":{"@private/*":["private/*"]},"rootDirs":["../src","../src/public","../generated"]}}"#,
        ),
        (
            "config/strict.json",
            r#"{"compilerOptions":{"strict":true}}"#,
        ),
        (
            "tsconfig.json",
            r#"{"extends":["./config/base.json","./config/strict.json"]}"#,
        ),
        (
            "src/public/entry.ts",
            "import '../shared'; import '@private/value'; import './nested'; import './own'; import './style.css';\n",
        ),
        ("generated/shared.ts", "export const generated = true;\n"),
        ("generated/nested.ts", "export {};\n"),
        ("generated/style.css.d.ts", "export {};\n"),
        ("generated/own.ts", "export {};\n"),
        ("src/public/own.ts", "export {};\n"),
        ("private/value.ts", "export const secret = true;\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        targets_from(&output, "src/public/entry.ts"),
        [
            "generated/nested.ts",
            "generated/shared.ts",
            "generated/style.css.d.ts",
            "private/value.ts",
            "src/public/own.ts",
        ]
    );
}

#[test]
fn module_suffixes_preserve_aliases_extension_priority_explicit_imports_and_indexes() {
    let root = repository(&[
        (
            "config/base.json",
            r#"{"compilerOptions":{"moduleSuffixes":[".native",""]}}"#,
        ),
        (
            "tsconfig.json",
            r#"{"extends":"./config/base.json","compilerOptions":{"baseUrl":".","paths":{"@private/*":["private/*"]}}}"#,
        ),
        (
            "src/entry.ts",
            "import '@private/value'; import '@private/explicit.js'; import '@private/folder'; import '@private/priority';\n",
        ),
        ("private/value.ts", "export {};\n"),
        ("private/value.native.ts", "export {};\n"),
        ("private/explicit.native.ts", "export {};\n"),
        ("private/folder/index.native.ts", "export {};\n"),
        ("private/priority.ts", "export {};\n"),
        ("private/priority.native.tsx", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        targets_from(&output, "src/entry.ts"),
        [
            "private/explicit.native.ts",
            "private/folder/index.native.ts",
            "private/priority.ts",
            "private/value.native.ts",
        ]
    );
}

#[test]
fn module_suffixes_without_an_empty_suffix_never_use_an_unsuffixed_file() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"extends":"./config/base.json","compilerOptions":{"moduleSuffixes":[".native"]}}"#,
        ),
        ("config/base.json", "{}"),
        ("entry.ts", "import './value'; import './native';\n"),
        ("value.ts", "export {};\n"),
        ("native.native.ts", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(
        output.edges,
        [edge("entry.ts", "native.native.ts", SideEffectImport)]
    );
    assert_eq!(
        output.gaps,
        [gap("entry.ts", "unresolved side-effect-import: ./value")]
    );

    write_files(
        root.path(),
        &[
            (
                "tsconfig.json",
                r#"{"compilerOptions":{"moduleSuffixes":[]}}"#,
            ),
            ("entry.ts", "import './value';\n"),
        ],
    );
    let output = analyze(root.path());
    assert_eq!(output.gaps, []);
    assert_eq!(targets_from(&output, "entry.ts"), ["value.ts"]);
}

#[test]
fn root_dirs_cannot_resolve_outside_the_snapshot() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"rootDirs":[".","../outside"]}}"#,
        ),
        ("entry.ts", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert!(has_gap(&output, "tsconfig.json", |reason| reason
        .starts_with("invalid TypeScript config:")));
}

fn extends_chain(length: usize, extends: impl Fn(usize) -> String) -> TempDir {
    let mut files = (0..length)
        .map(|index| {
            (
                format!("config/{index}.json"),
                format!(r#"{{"extends":{}}}"#, extends(index)),
            )
        })
        .collect::<Vec<_>>();
    files.push((format!("config/{length}.json"), "{}".into()));
    files.push((
        "tsconfig.json".into(),
        r#"{"extends":"./config/0.json"}"#.into(),
    ));
    files.push(("main.ts".into(), "import './value';\n".into()));
    files.push(("value.ts".into(), "export {};\n".into()));
    let files = files
        .iter()
        .map(|(path, contents)| (path.as_str(), contents.as_str()))
        .collect::<Vec<_>>();
    repository(&files)
}

#[test]
fn shared_extended_configs_are_loaded_once() {
    let root = extends_chain(40, |index| {
        let next = index + 1;
        format!(r#"["./{next}.json","./{next}.json"]"#)
    });

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("main.ts", "value.ts", SideEffectImport)]
    );
}

#[test]
fn deep_extended_config_chains_become_a_config_gap() {
    let root = extends_chain(70, |index| format!(r#""./{}.json""#, index + 1));

    let output = analyze(root.path());

    assert_eq!(
        output.gaps,
        [gap(
            "tsconfig.json",
            "invalid TypeScript config: extended config depth limit exceeded (64)"
        )]
    );
    assert_eq!(
        output.edges,
        [edge("main.ts", "value.ts", SideEffectImport)]
    );
}
