use crate::{
    findings::Findings,
    limits,
    snapshot::{Inventory, SourceFile},
};
use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, CallExpression, ExportAllDeclaration, ExportFromDeclaration, Expression,
    ImportDeclaration, ImportDeclarationSpecifier, ImportExpression, TSImportEqualsDeclaration,
    TSImportType, TSModuleReference,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{ParseOptions, Parser};
use oxc_span::SourceType;
use rayon::prelude::*;
use scope_domain::dependency_analysis::{DependencyEdgeKind, DependencyGap};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
};

const MAX_SYNTAX_ERROR_CHARS: usize = 400;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Reference {
    pub(crate) specifier: String,
    pub(crate) kind: DependencyEdgeKind,
}

#[derive(Default)]
pub(crate) struct Scan {
    pub(crate) analyzed_files: Vec<String>,
    pub(crate) resolvable_files: Vec<String>,
    pub(crate) references: HashMap<String, Vec<Reference>>,
    references_exceeded: bool,
}

enum SourceScan {
    Unreadable(String),
    Parsed {
        references: BTreeSet<Reference>,
        syntax_error: Option<String>,
        gap_reasons: BTreeSet<&'static str>,
    },
}

pub(crate) fn scan_sources(inventory: &Inventory, findings: &mut Findings) -> Scan {
    let mut scan = Scan::default();
    let mut references_left = limits::MAX_REFERENCES;
    for chunk in source_chunks(&inventory.sources) {
        let scanned = chunk
            .par_iter()
            .map(|source| (&source.path, scan_source(inventory, &source.path)))
            .collect::<Vec<_>>();
        for (path, outcome) in scanned {
            scan.record(path, outcome, findings, &mut references_left);
        }
    }
    if scan.references_exceeded {
        findings.add_repository_gap(format!(
            "dependency reference limit exceeded ({})",
            limits::MAX_REFERENCES
        ));
    }
    scan
}

fn source_chunks(sources: &[SourceFile]) -> Vec<&[SourceFile]> {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut bytes = 0;
    for (index, source) in sources.iter().enumerate() {
        if index > start && bytes + source.bytes > limits::MAX_CONCURRENT_SOURCE_BYTES {
            chunks.push(&sources[start..index]);
            start = index;
            bytes = 0;
        }
        bytes += source.bytes;
    }
    if start < sources.len() {
        chunks.push(&sources[start..]);
    }
    chunks
}

impl Scan {
    fn record(
        &mut self,
        path: &str,
        outcome: SourceScan,
        findings: &mut Findings,
        references_left: &mut usize,
    ) {
        match outcome {
            SourceScan::Unreadable(error) => {
                findings.add_gap(gap(path, format!("source file is unreadable: {error}")));
            }
            SourceScan::Parsed {
                references,
                syntax_error,
                gap_reasons,
            } => {
                self.analyzed_files.push(path.to_owned());
                match syntax_error {
                    Some(error) => findings.add_gap(gap(path, format!("syntax error: {error}"))),
                    None => self.resolvable_files.push(path.to_owned()),
                }
                for reason in gap_reasons {
                    findings.add_gap(gap(path, reason.into()));
                }
                if self.references_exceeded || references.len() > *references_left {
                    self.references_exceeded = true;
                    return;
                }
                *references_left -= references.len();
                self.references
                    .insert(path.to_owned(), references.into_iter().collect());
            }
        }
    }
}

fn gap(path: &str, reason: String) -> DependencyGap {
    DependencyGap {
        path: path.to_owned(),
        reason,
    }
}

fn scan_source(inventory: &Inventory, path: &str) -> SourceScan {
    let bytes = match inventory
        .absolute(path)
        .map_err(String::from)
        .and_then(|absolute| fs::read(absolute).map_err(|error| error.to_string()))
    {
        Ok(bytes) => bytes,
        Err(error) => return SourceScan::Unreadable(error),
    };
    let text = String::from_utf8_lossy(&bytes);
    let allocator = Allocator::default();
    let source_type = SourceType::unambiguous()
        .with_typescript(true)
        .with_jsx(path.ends_with('x'));
    let parsed = Parser::new(&allocator, &text, source_type)
        .with_options(ParseOptions {
            preserve_parens: false,
            ..ParseOptions::default()
        })
        .parse();
    let syntax_error = parsed.diagnostics.iter().next().map(|error| {
        error
            .to_string()
            .chars()
            .take(MAX_SYNTAX_ERROR_CHARS)
            .collect()
    });
    let mut collector = ReferenceCollector::default();
    collector.visit_program(&parsed.program);
    SourceScan::Parsed {
        references: collector.references,
        syntax_error,
        gap_reasons: collector.gap_reasons,
    }
}

#[derive(Default)]
struct ReferenceCollector {
    references: BTreeSet<Reference>,
    gap_reasons: BTreeSet<&'static str>,
}

impl ReferenceCollector {
    fn add(&mut self, specifier: Option<&str>, kind: DependencyEdgeKind) {
        if let Some(specifier) = specifier {
            self.references.insert(Reference {
                specifier: specifier.to_owned(),
                kind,
            });
        }
    }

    fn add_literal(
        &mut self,
        expression: Option<&Expression>,
        kind: DependencyEdgeKind,
        non_literal: &'static str,
    ) {
        match expression.and_then(literal_text) {
            Some(specifier) => self.add(Some(specifier), kind),
            None => {
                self.gap_reasons.insert(non_literal);
            }
        }
    }
}

fn literal_text<'a>(expression: &'a Expression) -> Option<&'a str> {
    match expression {
        Expression::StringLiteral(literal) => literal.value.as_str(),
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
            template.quasis.first()?.value.cooked?.as_str()
        }
        _ => None,
    }
}

impl<'a> Visit<'a> for ReferenceCollector {
    fn visit_import_declaration(&mut self, declaration: &ImportDeclaration<'a>) {
        let kind = match &declaration.specifiers {
            Some(specifiers) if !specifiers.is_empty() => {
                let type_only = declaration.import_kind.is_type()
                    || specifiers.iter().all(|specifier| {
                        matches!(
                            specifier,
                            ImportDeclarationSpecifier::ImportSpecifier(specifier)
                                if specifier.import_kind.is_type()
                        )
                    });
                if type_only {
                    DependencyEdgeKind::TypeImport
                } else {
                    DependencyEdgeKind::Import
                }
            }
            _ => DependencyEdgeKind::SideEffectImport,
        };
        self.add(declaration.source.value.as_str(), kind);
        walk::walk_import_declaration(self, declaration);
    }

    fn visit_export_from_declaration(&mut self, declaration: &ExportFromDeclaration<'a>) {
        self.add(
            declaration.source.value.as_str(),
            re_export_kind(declaration.export_kind.is_type()),
        );
        walk::walk_export_from_declaration(self, declaration);
    }

    fn visit_export_all_declaration(&mut self, declaration: &ExportAllDeclaration<'a>) {
        self.add(
            declaration.source.value.as_str(),
            re_export_kind(declaration.export_kind.is_type()),
        );
        walk::walk_export_all_declaration(self, declaration);
    }

    fn visit_ts_import_equals_declaration(&mut self, declaration: &TSImportEqualsDeclaration<'a>) {
        if let TSModuleReference::ExternalModuleReference(reference) = &declaration.module_reference
        {
            self.add(
                reference.expression.value.as_str(),
                DependencyEdgeKind::Require,
            );
        }
        walk::walk_ts_import_equals_declaration(self, declaration);
    }

    fn visit_ts_import_type(&mut self, import: &TSImportType<'a>) {
        self.add(import.source.value.as_str(), DependencyEdgeKind::TypeImport);
        walk::walk_ts_import_type(self, import);
    }

    fn visit_import_expression(&mut self, import: &ImportExpression<'a>) {
        self.add_literal(
            Some(&import.source),
            DependencyEdgeKind::DynamicImport,
            "non-literal dynamic import",
        );
        walk::walk_import_expression(self, import);
    }

    fn visit_call_expression(&mut self, call: &CallExpression<'a>) {
        if !call.optional
            && matches!(&call.callee, Expression::Identifier(callee) if callee.name == "require")
        {
            self.add_literal(
                call.arguments.first().and_then(Argument::as_expression),
                DependencyEdgeKind::Require,
                "non-literal require",
            );
        }
        walk::walk_call_expression(self, call);
    }
}

fn re_export_kind(type_only: bool) -> DependencyEdgeKind {
    if type_only {
        DependencyEdgeKind::TypeReExport
    } else {
        DependencyEdgeKind::ReExport
    }
}
