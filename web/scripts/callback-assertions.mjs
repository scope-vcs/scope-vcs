import { parse } from '@babel/parser'
import { walk } from './resource-boundary.mjs'

export function callbackAssertionViolations(filename, source) {
  const file = parse(source, { sourceType: 'module', plugins: ['typescript', 'jsx'] })
  const assertions = new Set()
  const failures = new Set()
  const tests = new Set()
  for (const node of file.program.body) {
    if (node.type !== 'ImportDeclaration') continue
    for (const binding of node.specifiers) {
      if (/^(node:)?assert(?:\/strict)?$/.test(node.source.value)) {
        if (binding.type !== 'ImportSpecifier' || binding.imported.name === 'strict') assertions.add(binding.local.name)
        else if (binding.imported.name === 'fail') failures.add(binding.local.name)
      }
      if (node.source.value === 'node:test' &&
        (binding.type === 'ImportDefaultSpecifier' || binding.imported?.name === 'test' || binding.imported?.name === 'it')) {
        tests.add(binding.local.name)
      }
    }
  }
  const testBodies = new Set()
  walk(file.program, (node) => {
    if (node.type === 'CallExpression' && tests.has(node.callee.name)) testBodies.add(node.arguments.at(-1))
  })

  const violations = []
  function visit(node, parent, boundary) {
    if (!node || typeof node !== 'object') return
    if (Array.isArray(node)) {
      for (const child of node) visit(child, parent, boundary)
      return
    }
    if (typeof node.type !== 'string') return
    if (/^(ArrowFunctionExpression|FunctionExpression|FunctionDeclaration|ObjectMethod)$/.test(node.type)) {
      boundary = testBodies.has(node) || node.type === 'FunctionDeclaration' && parent?.type === 'Program'
    }
    const failure = node.type === 'Identifier' && failures.has(node.name) && parent?.type !== 'ImportSpecifier'
      || node.type === 'MemberExpression' && assertions.has(node.object.name) &&
        (node.computed ? node.property.value : node.property.name) === 'fail'
    if (failure && !(boundary && parent?.type === 'CallExpression' && parent.callee === node)) {
      violations.push(`${filename}:${node.loc.start.line}: assert.fail in an injected callback can be caught by the code under test; use node:test mock.fn() and assert mock.callCount() after awaiting the operation`)
    }
    for (const [key, child] of Object.entries(node)) {
      if (!['loc', 'extra', 'comments'].includes(key)) visit(child, node, boundary)
    }
  }
  visit(file.program, null, false)
  return violations
}
