import { readFile } from "node:fs/promises";

import { parse } from "@babel/parser";

import { absoluteSnapshotPath } from "./snapshot.mjs";

function literalText(node) {
  if (node?.type === "StringLiteral") return node.value;
  if (node?.type === "TemplateLiteral" && node.expressions.length === 0) {
    return node.quasis[0].value.cooked;
  }
  return null;
}

function addReference(references, specifier, kind) {
  const key = `${specifier}\0${kind}`;
  if (!references.has(key)) references.set(key, { kind, specifier });
}

function walk(node, visit) {
  if (!node || typeof node !== "object") return;
  if (Array.isArray(node)) {
    for (const child of node) walk(child, visit);
    return;
  }
  if (typeof node.type !== "string") return;
  visit(node);
  for (const [key, value] of Object.entries(node)) {
    if (key !== "loc" && key !== "extra" && key !== "comments") walk(value, visit);
  }
}

function scanNode(node, references, gapReasons) {
  if (node.type === "ImportDeclaration") {
    const kind = node.specifiers.length === 0 ? "side-effect-import"
      : node.importKind === "type" || node.specifiers.every((specifier) => specifier.importKind === "type")
        ? "type-import" : "import";
    addReference(references, node.source.value, kind);
  } else if ((node.type === "ExportNamedDeclaration" || node.type === "ExportAllDeclaration") && node.source) {
    addReference(references, node.source.value, node.exportKind === "type" ? "type-re-export" : "re-export");
  } else if (node.type === "TSImportEqualsDeclaration" && node.moduleReference.type === "TSExternalModuleReference") {
    const specifier = literalText(node.moduleReference.expression);
    if (specifier === null) gapReasons.add("non-literal require");
    else addReference(references, specifier, "require");
  } else if (node.type === "TSImportType") {
    const specifier = literalText(node.argument);
    if (specifier === null) gapReasons.add("non-literal type import");
    else addReference(references, specifier, "type-import");
  } else if (node.type === "ImportExpression" || node.type === "CallExpression" && node.callee.type === "Import") {
    const argument = node.type === "ImportExpression" ? node.source : node.arguments[0];
    const specifier = literalText(argument);
    if (specifier === null) gapReasons.add("non-literal dynamic import");
    else addReference(references, specifier, "dynamic-import");
  } else if (node.type === "CallExpression" && node.callee.type === "Identifier" && node.callee.name === "require") {
    const specifier = literalText(node.arguments[0]);
    if (specifier === null) gapReasons.add("non-literal require");
    else addReference(references, specifier, "require");
  }
}

export async function scanSources(root, sourcePaths) {
  const analyzedFiles = [];
  const resolvableFiles = [];
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
    const references = new Map();
    const gapReasons = new Set();
    try {
      const file = parse(text, {
        sourceType: "unambiguous",
        plugins: ["typescript", ...(path.endsWith("x") ? ["jsx"] : [])],
        errorRecovery: true,
      });
      if (file.errors.length) {
        gaps.push({ path, reason: `syntax error: ${file.errors[0].message.slice(0, 400)}` });
      } else {
        resolvableFiles.push(path);
      }
      walk(file.program, (node) => scanNode(node, references, gapReasons));
    } catch (error) {
      gaps.push({ path, reason: `syntax error: ${error.message.slice(0, 400)}` });
    }
    referencesBySource.set(path, [...references.values()]);
    for (const reason of gapReasons) gaps.push({ path, reason });
  }

  return { analyzedFiles, resolvableFiles, gaps, referencesBySource };
}
