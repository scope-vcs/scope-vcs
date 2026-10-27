mod support;

use scope_domain::dependency_analysis::DependencyEdgeKind::*;
use std::fs;
use support::*;
use tempfile::TempDir;

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
fn inherited_paths_use_a_childs_explicit_base_url_as_typescript_does() {
    for base_url in [".", ""] {
        let root = repository(&[
            (
                "config/base.json",
                r#"{"compilerOptions":{"baseUrl":".","paths":{"@x/*":["src/*"]}}}"#,
            ),
            (
                "app/override.json",
                &format!(
                    r#"{{"extends":"../config/base.json","compilerOptions":{{"baseUrl":"{base_url}"}}}}"#
                ),
            ),
            ("app/tsconfig.json", r#"{"extends":"./override.json"}"#),
            ("config/src/value.ts", "export const value = 1;\n"),
            ("app/src/value.ts", "export const value = 1;\n"),
            (
                "app/main.ts",
                "import { value } from '@x/value'; import 'src/value'; export { value };\n",
            ),
        ]);

        let output = analyze(root.path());

        assert_eq!(output.gaps, [], "baseUrl: {base_url:?}");
        let mut expected = vec![edge("app/main.ts", "app/src/value.ts", Import)];
        if !base_url.is_empty() {
            expected.push(edge("app/main.ts", "app/src/value.ts", SideEffectImport));
        }
        assert_eq!(output.edges, expected, "baseUrl: {base_url:?}");
    }
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
fn installed_extended_configs_never_reach_outside_the_snapshot() {
    let parent = TempDir::new().unwrap();
    let outside = parent.path().join("outside.json");
    let redirect = parent.path().join("repo/redirect.ts");
    write_files(
        parent.path(),
        &[
            (
                "outside.json",
                &format!(
                    r#"{{"compilerOptions":{{"paths":{{"@x":["{}"]}}}}}}"#,
                    redirect.display()
                ),
            ),
            ("repo/tsconfig.json", r#"{"extends":"base"}"#),
            (
                "repo/base.json",
                r#"{"compilerOptions":{"paths":{"@x":["./value.ts"]}}}"#,
            ),
            (
                "repo/node_modules/base/package.json",
                r#"{"name":"base","exports":"./tsconfig.json"}"#,
            ),
            (
                "repo/node_modules/base/tsconfig.json",
                &format!(r#"{{"extends":"{}"}}"#, outside.display()),
            ),
            ("repo/entry.ts", "import { value } from '@x';\n"),
            ("repo/value.ts", "export {};\n"),
            ("repo/redirect.ts", "export {};\n"),
        ],
    );

    let output = analyze(&parent.path().join("repo"));

    assert_eq!(output.gaps, []);
    assert_eq!(output.edges, [edge("entry.ts", "value.ts", Import)]);
}

#[test]
fn config_fields_outside_module_resolution_keep_every_edge() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"include":"bad","files":"bad","compilerOptions":{"strict":"no","target":null,"paths":{"@x":["value.ts"]}}}"#,
        ),
        (
            "entry.ts",
            "import { value } from '@x'; import './value';\n",
        ),
        ("value.ts", "export const value = 1;\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [
            edge("entry.ts", "value.ts", Import),
            edge("entry.ts", "value.ts", SideEffectImport),
        ]
    );
}

#[test]
fn inherited_paths_without_base_url_resolve_from_the_consuming_config() {
    let root = repository(&[
        ("tsconfig.json", r#"{"extends":"./config/base.json"}"#),
        (
            "config/base.json",
            r#"{"compilerOptions":{"paths":{"@x":["./value.ts"]}}}"#,
        ),
        ("config/value.ts", "export {};\n"),
        ("value.ts", "export {};\n"),
        ("entry.ts", "import '@x';\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("entry.ts", "value.ts", SideEffectImport)]
    );
}

#[test]
fn duplicate_config_keys_use_the_last_value_like_json_parse() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"strict":true},"compilerOptions":{"paths":{"@x":["missing.ts"],"@x":["value.ts"]}}}"#,
        ),
        ("entry.ts", "import '@x';\n"),
        ("value.ts", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("entry.ts", "value.ts", SideEffectImport)]
    );
}

#[test]
fn repeated_extended_configs_contribute_only_on_their_first_visit() {
    let diamond = repository(&[
        ("tsconfig.json", r#"{"extends":["./a.json","./b.json"]}"#),
        (
            "a.json",
            r#"{"extends":"./base.json","compilerOptions":{"paths":{"@x":["value.ts"]}}}"#,
        ),
        ("b.json", r#"{"extends":"./base.json"}"#),
        (
            "base.json",
            r#"{"compilerOptions":{"paths":{"@x":["other.ts"]}}}"#,
        ),
        ("entry.ts", "import '@x';\n"),
        ("value.ts", "export {};\n"),
        ("other.ts", "export {};\n"),
    ]);
    let duplicate = repository(&[
        (
            "tsconfig.json",
            r#"{"extends":["./base.json","./other.json","./base.json"]}"#,
        ),
        (
            "base.json",
            r#"{"compilerOptions":{"paths":{"@x":["value.ts"]}}}"#,
        ),
        (
            "other.json",
            r#"{"compilerOptions":{"paths":{"@x":["other.ts"]}}}"#,
        ),
        ("entry.ts", "import '@x';\n"),
        ("value.ts", "export {};\n"),
        ("other.ts", "export {};\n"),
    ]);

    for (root, target) in [(diamond, "value.ts"), (duplicate, "other.ts")] {
        let output = analyze(root.path());

        assert_eq!(output.gaps, []);
        assert_eq!(
            output.edges,
            [edge("entry.ts", target, SideEffectImport)],
            "{target}"
        );
    }
}

#[test]
fn path_aliases_match_requests_without_their_query() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@x":["value.ts"]}}}"#,
        ),
        ("entry.ts", "import '@x?raw';\n"),
        ("value.ts", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("entry.ts", "value.ts", SideEffectImport)]
    );
}

#[test]
fn wildcard_alias_targets_normalize_parent_segments() {
    let root = repository(&[
        (
            "tsconfig.json",
            r#"{"compilerOptions":{"paths":{"@x/*":["lib/*"]}}}"#,
        ),
        ("entry.ts", "import '@x/missing/../value';\n"),
        ("lib/value.ts", "export {};\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("entry.ts", "lib/value.ts", SideEffectImport)]
    );
}

#[test]
fn alias_targets_are_aliased_again() {
    let root = TempDir::new().unwrap();
    let library = fs::canonicalize(root.path()).unwrap().join("lib");
    write_files(
        root.path(),
        &[
            (
                "tsconfig.json",
                &format!(
                    r#"{{"compilerOptions":{{"paths":{{"@x":["lib"],"{}":["value.ts"]}}}}}}"#,
                    library.display()
                ),
            ),
            ("entry.ts", "import '@x';\n"),
            ("value.ts", "export {};\n"),
        ],
    );

    let output = analyze(root.path());

    assert_eq!(output.gaps, []);
    assert_eq!(
        output.edges,
        [edge("entry.ts", "value.ts", SideEffectImport)]
    );
}

#[test]
fn hash_comments_make_a_config_invalid_without_losing_relative_edges() {
    let root = repository(&[
        (
            "tsconfig.json",
            "{ # comment\n \"compilerOptions\":{\"paths\":{\"@x\":[\"value.ts\"]}}}",
        ),
        (
            "entry.ts",
            "import { value } from '@x'; import './value';\n",
        ),
        ("value.ts", "export const value = 1;\n"),
    ]);

    let output = analyze(root.path());

    assert_eq!(
        output.gaps,
        [gap(
            "tsconfig.json",
            "invalid TypeScript config: malformed JSONC at tsconfig.json"
        )]
    );
    assert_eq!(
        output.edges,
        [edge("entry.ts", "value.ts", SideEffectImport)]
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
