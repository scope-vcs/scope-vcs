use crate::{
    config::ResolutionConfig,
    findings::Findings,
    limits,
    paths::{is_outside, resolve_lexically},
    scan::Reference,
    snapshot::Inventory,
    snapshot_file_system::SnapshotFileSystem,
};
use oxc_resolver::{ResolveOptions, ResolverGeneric};
use rayon::prelude::*;
use scope_domain::dependency_analysis::{DependencyEdge, DependencyEdgeKind, DependencyGap};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

type Resolver = ResolverGeneric<SnapshotFileSystem>;

const MAX_ALIAS_DEPTH: usize = 16;
const MAX_ALIAS_EXPANSIONS: usize = 64;

#[derive(Default)]
struct AliasExpansion {
    chain: Vec<String>,
    expansions: usize,
}

impl AliasExpansion {
    fn enter(&mut self, specifier: &str) -> bool {
        if self.chain.len() >= MAX_ALIAS_DEPTH
            || self.expansions >= MAX_ALIAS_EXPANSIONS
            || self.chain.iter().any(|seen| seen == specifier)
        {
            return false;
        }
        self.expansions += 1;
        self.chain.push(specifier.to_owned());
        true
    }

    fn leave(&mut self) {
        self.chain.pop();
    }
}

const EXTENSIONS: &[&str] = &[
    ".ts", ".tsx", ".d.ts", ".mts", ".d.mts", ".cts", ".d.cts", ".js", ".jsx", ".mjs", ".cjs",
    ".json",
];

pub(crate) struct GroupContext<'a> {
    pub(crate) inventory: &'a Inventory,
    pub(crate) internal_package_names: &'a HashSet<String>,
    pub(crate) references: &'a HashMap<String, Vec<Reference>>,
}

struct Resolvers<'a> {
    config: Option<&'a ResolutionConfig>,
    file_system: SnapshotFileSystem,
    by_mode: [Resolver; 4],
}

impl<'a> Resolvers<'a> {
    fn new(inventory: &Inventory, config: Option<&'a ResolutionConfig>) -> Self {
        let file_system = SnapshotFileSystem::new(
            inventory,
            config
                .map(|config| config.module_suffixes.clone())
                .unwrap_or_default(),
        );
        let base =
            Resolver::new_with_file_system(file_system.clone(), resolve_options(false, false));
        let by_mode = [
            base.clone_with_options(resolve_options(false, true)),
            base.clone_with_options(resolve_options(true, false)),
            base.clone_with_options(resolve_options(true, true)),
            base,
        ];
        Self {
            config,
            file_system,
            by_mode,
        }
    }

    fn resolve(&self, resolver: &Resolver, directory: &Path, specifier: &str) -> Option<String> {
        let mut aliased = AliasExpansion::default();
        self.resolve_configured(resolver, directory, specifier, &mut aliased)
            .or_else(|| {
                let config = self.config?;
                if !specifier.starts_with('.') {
                    return None;
                }
                config
                    .root_dir_candidates(&resolve_lexically(directory, specifier))
                    .into_iter()
                    .find_map(|candidate| {
                        self.resolve_configured(
                            resolver,
                            directory,
                            candidate.to_str()?,
                            &mut aliased,
                        )
                    })
            })
            .and_then(|resolved| self.file_system.snapshot_file(&resolved))
    }

    fn resolve_configured(
        &self,
        resolver: &Resolver,
        directory: &Path,
        specifier: &str,
        aliased: &mut AliasExpansion,
    ) -> Option<PathBuf> {
        let resolve = |specifier: &str| {
            resolver
                .resolve(directory, specifier)
                .ok()
                .map(|resolution| resolution.path().to_path_buf())
        };
        let Some(config) = self.config else {
            return resolve(specifier);
        };
        if let Some(targets) = config.alias_targets(specifier)
            && aliased.enter(specifier)
        {
            let resolved = targets
                .iter()
                .find_map(|target| self.resolve_configured(resolver, directory, target, aliased));
            aliased.leave();
            if resolved.is_some() {
                return resolved;
            }
        }
        if is_bare_specifier(specifier) {
            let in_module_directory = config
                .module_directories
                .iter()
                .find_map(|module_directory| resolve(module_directory.join(specifier).to_str()?));
            if in_module_directory.is_some() {
                return in_module_directory;
            }
        }
        resolve(specifier)
    }

    fn clear_cache(&self) {
        self.by_mode[0].clear_cache();
    }

    fn for_reference(&self, kind: DependencyEdgeKind, common_js_source: bool) -> &Resolver {
        let type_only = matches!(
            kind,
            DependencyEdgeKind::TypeImport | DependencyEdgeKind::TypeReExport
        );
        let require_mode = kind == DependencyEdgeKind::Require
            || common_js_source && kind != DependencyEdgeKind::DynamicImport;
        match (type_only, require_mode) {
            (false, true) => &self.by_mode[0],
            (true, false) => &self.by_mode[1],
            (true, true) => &self.by_mode[2],
            (false, false) => &self.by_mode[3],
        }
    }
}

fn resolve_options(type_only: bool, require_mode: bool) -> ResolveOptions {
    let module_condition = if require_mode { "require" } else { "import" };
    let condition_names = if type_only {
        vec!["types", module_condition, "node", "default"]
    } else {
        vec![module_condition, "node", "default"]
    };
    let main_fields = if type_only {
        vec!["types", "module", "main"]
    } else {
        vec!["module", "main"]
    };
    ResolveOptions {
        condition_names: strings(&condition_names),
        exports_fields: vec![vec!["exports".into()]],
        imports_fields: vec![vec!["imports".into()]],
        extensions: strings(EXTENSIONS),
        extension_alias: vec![
            (".js".into(), strings(&[".ts", ".tsx", ".d.ts", ".js"])),
            (".mjs".into(), strings(&[".mts", ".d.mts", ".mjs"])),
            (".cjs".into(), strings(&[".cts", ".d.cts", ".cjs"])),
        ],
        main_fields: strings(&main_fields),
        main_files: strings(&["index"]),
        symlinks: false,
        node_path: false,
        builtin_modules: false,
        ..ResolveOptions::default()
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

pub(crate) fn resolve_group(
    context: &GroupContext,
    config: Option<&ResolutionConfig>,
    sources: &[String],
    findings: &mut Findings,
) {
    let resolvers = Resolvers::new(context.inventory, config);
    let references = sources
        .iter()
        .flat_map(|source| {
            context
                .references
                .get(source)
                .into_iter()
                .flatten()
                .map(move |reference| (source, reference))
        })
        .collect::<Vec<_>>();
    for batch in references.chunks(limits::RESOLUTION_BATCH) {
        if !findings.within_limits() {
            return;
        }
        let batch_findings = batch
            .par_iter()
            .fold(
                Findings::default,
                |mut batch_findings, (source, reference)| {
                    resolve_reference(context, &resolvers, source, reference, &mut batch_findings);
                    batch_findings
                },
            )
            .reduce(Findings::default, |mut left, right| {
                left.merge(right);
                left
            });
        findings.merge(batch_findings);
        resolvers.clear_cache();
    }
}

fn resolve_reference(
    context: &GroupContext,
    resolvers: &Resolvers,
    source: &str,
    reference: &Reference,
    findings: &mut Findings,
) {
    let Reference { specifier, kind } = reference;
    if specifier.starts_with("node:") {
        return;
    }
    let root = &context.inventory.root;
    let unclaimed_bare = is_bare_specifier(specifier)
        && !resolvers
            .config
            .is_some_and(|config| config.claims(specifier))
        && !context
            .internal_package_names
            .contains(package_name(specifier));
    let common_js_source = source.ends_with(".cts") || source.ends_with(".cjs");
    let Ok(source_file) = context.inventory.absolute(source) else {
        return;
    };
    let source_directory = source_file.parent().unwrap_or(root);
    let mut gap = |reason: &str| {
        findings.add_gap(DependencyGap {
            path: source.to_owned(),
            reason: format!("{reason}: {specifier}"),
        });
    };

    let resolver = resolvers.for_reference(*kind, common_js_source);
    let Some(target_path) = resolvers.resolve(resolver, source_directory, specifier) else {
        if unclaimed_bare {
            return;
        }
        if names_path_outside(root, source_directory, specifier) {
            gap("resolved target escapes snapshot");
        } else {
            gap(&format!("unresolved {}", kind_name(*kind)));
        }
        return;
    };
    findings.add_edge(DependencyEdge {
        source_path: source.to_owned(),
        target_path,
        kind: *kind,
    });
}

fn names_path_outside(root: &Path, directory: &Path, specifier: &str) -> bool {
    (specifier.starts_with(['.', '/']) || Path::new(specifier).is_absolute())
        && is_outside(root, &resolve_lexically(directory, specifier))
}

pub(crate) fn kind_name(kind: DependencyEdgeKind) -> &'static str {
    match kind {
        DependencyEdgeKind::Import => "import",
        DependencyEdgeKind::TypeImport => "type-import",
        DependencyEdgeKind::ReExport => "re-export",
        DependencyEdgeKind::TypeReExport => "type-re-export",
        DependencyEdgeKind::DynamicImport => "dynamic-import",
        DependencyEdgeKind::Require => "require",
        DependencyEdgeKind::SideEffectImport => "side-effect-import",
    }
}

fn is_bare_specifier(specifier: &str) -> bool {
    !specifier.starts_with(['.', '/', '#'])
}

fn package_name(specifier: &str) -> &str {
    let segments = if specifier.starts_with('@') { 2 } else { 1 };
    match specifier.match_indices('/').nth(segments - 1) {
        Some((index, _)) => &specifier[..index],
        None => specifier,
    }
}
