mod support;

use scope_domain::dependency_analysis::{DEPENDENCY_ANALYZER_VERSION, DependencyEdgeKind::*};
use support::*;
use tempfile::TempDir;

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
