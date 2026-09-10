import { readFile } from "node:fs/promises";

import ts from "typescript";

import { absoluteSnapshotPath } from "./snapshot.mjs";

function literalText(node) {
  if (ts.isStringLiteralLike(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
    return node.text;
  }
  return null;
}

function importDeclarationKind(node) {
  if (!node.importClause) return "side-effect-import";
  if (node.importClause.isTypeOnly) return "type-import";
  if (
    node.importClause.namedBindings &&
    ts.isNamedImports(node.importClause.namedBindings) &&
    node.importClause.namedBindings.elements.length > 0 &&
    node.importClause.namedBindings.elements.every((element) => element.isTypeOnly)
  ) {
    return "type-import";
  }
  return "import";
}

function addReference(references, specifier, kind) {
  const key = `${specifier}\0${kind}`;
  if (!references.has(key)) references.set(key, { kind, specifier });
}

function scanNode(node, references, gapReasons) {
  if (ts.isImportDeclaration(node)) {
    const specifier = literalText(node.moduleSpecifier);
    if (specifier === null) gapReasons.add("non-literal import declaration");
    else addReference(references, specifier, importDeclarationKind(node));
  } else if (ts.isExportDeclaration(node) && node.moduleSpecifier) {
    const specifier = literalText(node.moduleSpecifier);
    if (specifier === null) gapReasons.add("non-literal re-export");
    else addReference(references, specifier, node.isTypeOnly ? "type-re-export" : "re-export");
  } else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) {
    const specifier = node.moduleReference.expression
      ? literalText(node.moduleReference.expression)
      : null;
    if (specifier === null) gapReasons.add("non-literal require");
    else addReference(references, specifier, "require");
  } else if (ts.isImportTypeNode(node)) {
    const specifier = ts.isLiteralTypeNode(node.argument)
      ? literalText(node.argument.literal)
      : null;
    if (specifier === null) gapReasons.add("non-literal type import");
    else addReference(references, specifier, "type-import");
  } else if (ts.isCallExpression(node) && node.expression.kind === ts.SyntaxKind.ImportKeyword) {
    const specifier = node.arguments.length > 0 ? literalText(node.arguments[0]) : null;
    if (specifier === null) gapReasons.add("non-literal dynamic import");
    else addReference(references, specifier, "dynamic-import");
  } else if (
    ts.isCallExpression(node) &&
    ts.isIdentifier(node.expression) &&
    node.expression.text === "require"
  ) {
    const specifier = node.arguments.length > 0 ? literalText(node.arguments[0]) : null;
    if (specifier === null) gapReasons.add("non-literal require");
    else addReference(references, specifier, "require");
  }

  ts.forEachChild(node, (child) => scanNode(child, references, gapReasons));
}

function diagnosticText(diagnostic) {
  return ts.flattenDiagnosticMessageText(diagnostic.messageText, " ").slice(0, 400);
}

export async function scanSources(root, sourcePaths) {
  const analyzedFiles = [];
  const cruisableFiles = [];
  const gaps = [];
  const referencesBySource = new Map();

  for (const path of sourcePaths) {
    let text;
    try {
      text = await readFile(absoluteSnapshotPath(root, path), "utf8");
    } catch (error) {
      gaps.push({ path, reason: `source file is unreadable: ${error.message}` });
      continue;
    }

    analyzedFiles.push(path);
    const sourceFile = ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true);
    const parseDiagnostics = sourceFile.parseDiagnostics ?? [];
    if (parseDiagnostics.length > 0) {
      gaps.push({ path, reason: `syntax error: ${diagnosticText(parseDiagnostics[0])}` });
    } else {
      cruisableFiles.push(path);
    }

    const references = new Map();
    const gapReasons = new Set();
    scanNode(sourceFile, references, gapReasons);
    referencesBySource.set(path, [...references.values()]);
    for (const reason of gapReasons) gaps.push({ path, reason });
  }

  return { analyzedFiles, cruisableFiles, gaps, referencesBySource };
}
