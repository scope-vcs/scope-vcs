import { parse } from '@babel/parser'

function exportedNames(source) {
  const file = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] })
  const values = new Set()
  const functions = new Set()
  for (const node of file.program.body) {
    if (node.type !== 'ExportNamedDeclaration') continue
    for (const specifier of node.specifiers) {
      if (node.exportKind !== 'type' && specifier.exportKind !== 'type') {
        values.add(specifier.exported.name ?? specifier.exported.value)
      }
    }
    const declaration = node.declaration
    if (!declaration || declaration.declare) continue
    if (declaration.type === 'FunctionDeclaration' && declaration.id) {
      values.add(declaration.id.name)
      functions.add(declaration.id.name)
    }
    if (declaration.type === 'VariableDeclaration') {
      for (const declarator of declaration.declarations) {
        if (declarator.id.type === 'Identifier') values.add(declarator.id.name)
      }
    }
  }
  return { values, functions }
}

export function conventionViolations(path, source) {
  const { values, functions } = exportedNames(source)
  const violations = []
  if (path.startsWith('src/routes/') && path.endsWith('.tsx') && !values.has('Route')) {
    violations.push(`${path}: route file must export Route`)
  }
  const match = /^src\/features\/[^/]+\/([^/]+)-page\.tsx$/.exec(path)
  if (match) {
    const expected = match[1].split('-').map((part) => part[0].toUpperCase() + part.slice(1)).join('') + 'Page'
    if (!functions.has(expected)) violations.push(`${path}: feature page must export function ${expected}`)
  }
  return violations
}
