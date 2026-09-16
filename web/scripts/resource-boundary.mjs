import ts from 'typescript'

// Catch direct or locally wrapped server reads inside component effects. Resource
// owners and event coordinators can manage effects; rendering code should consume them.
const isApiModule = (module) => /(?:^|\/)api\//.test(module)
const isRouteModule = (module) => /(?:^|\/)routes\//.test(module)
const LOADER = /^load[A-Z]/

export function resourceBoundaryViolations(filename, source) {
  const file = ts.createSourceFile(filename, source, ts.ScriptTarget.Latest, true, filename.endsWith('x') ? ts.ScriptKind.TSX : ts.ScriptKind.TS)
  const effectNames = new Set()
  const readNames = new Set(['fetch'])
  // Namespace alias -> member filter; null means every member call reads the server.
  const readNamespaces = new Map()
  const functions = new Map()
  for (const node of file.statements) {
    if (!ts.isImportDeclaration(node) || !ts.isStringLiteral(node.moduleSpecifier)) continue
    if (node.importClause?.isTypeOnly) continue
    const module = node.moduleSpecifier.text
    // Anything an api module exports is a server read; route modules only expose loaders.
    const api = isApiModule(module)
    const route = !api && isRouteModule(module)
    if (api && node.importClause?.name) readNames.add(node.importClause.name.text)
    const bindings = node.importClause?.namedBindings
    if (bindings && ts.isNamespaceImport(bindings)) {
      if (api) readNamespaces.set(bindings.name.text, null)
      else if (route) readNamespaces.set(bindings.name.text, LOADER)
    }
    if (bindings && ts.isNamedImports(bindings)) {
      for (const element of bindings.elements) {
        if (element.isTypeOnly) continue
        const imported = element.propertyName?.text ?? element.name.text
        if (module === 'react' && /^(useEffect|useLayoutEffect)$/.test(imported)) effectNames.add(element.name.text)
        if (api || (route && LOADER.test(imported))) readNames.add(element.name.text)
      }
    }
  }
  const collect = (node) => {
    if (ts.isFunctionDeclaration(node) && node.name) functions.set(node.name.text, node)
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer) {
      const value = node.initializer
      if (ts.isArrowFunction(value) || ts.isFunctionExpression(value)) functions.set(node.name.text, value)
      if (ts.isCallExpression(value) && value.arguments[0] && (ts.isArrowFunction(value.arguments[0]) || ts.isFunctionExpression(value.arguments[0]))) {
        functions.set(node.name.text, value.arguments[0])
      }
    }
    ts.forEachChild(node, collect)
  }
  collect(file)
  const reads = (node, visited = new Set()) => {
    if (!node) return false
    if (ts.isIdentifier(node) && functions.has(node.text) && !visited.has(node.text)) {
      visited.add(node.text)
      return reads(functions.get(node.text), visited)
    }
    if (ts.isCallExpression(node)) {
      if (ts.isIdentifier(node.expression) && readNames.has(node.expression.text)) return true
      if (ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)) {
        const member = readNamespaces.get(node.expression.expression.text)
        if (member === null || member?.test(node.expression.name.text)) return true
      }
      if (ts.isPropertyAccessExpression(node.expression) && node.expression.name.text === 'fetch' && /^(window|globalThis)$/.test(node.expression.expression.getText(file))) return true
    }
    let found = false
    ts.forEachChild(node, (child) => { found ||= reads(child, visited) })
    return found
  }
  const violations = []
  const inspect = (node) => {
    if (ts.isCallExpression(node) && (
      ts.isIdentifier(node.expression) && effectNames.has(node.expression.text) ||
      ts.isPropertyAccessExpression(node.expression) && /^(useEffect|useLayoutEffect)$/.test(node.expression.name.text)
    ) && reads(node.arguments[0])) {
      const { line } = file.getLineAndCharacterOfPosition(node.getStart(file))
      violations.push(`${filename}:${line + 1}: component effect owns a server read; use the existing resource/cache owner`)
    }
    ts.forEachChild(node, inspect)
  }
  inspect(file)
  return violations
}
