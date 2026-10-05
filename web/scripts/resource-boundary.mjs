import { parse } from '@babel/parser'

const isApiModule = (module) => /(?:^|\/)api\//.test(module)
const isRouteModule = (module) => /(?:^|\/)routes\//.test(module)
const isResourceModule = (module) => /(?:^|\/)[^/]*(?:resource|cache)$/.test(module)
const LOADER = /^load[A-Z]/
const RESOURCE_WRITE = /^(write|seed|writeIfUnchanged)$/

export function walk(node, visit) {
  if (!node || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (const child of node) walk(child, visit)
    return
  }
  if (typeof node.type !== 'string') return
  visit(node)
  for (const [key, value] of Object.entries(node)) {
    if (key !== 'loc' && key !== 'extra' && key !== 'comments') walk(value, visit)
  }
}

function isFunction(node) {
  return node?.type === 'ArrowFunctionExpression' || node?.type === 'FunctionExpression'
}

function calledMember(node) {
  return node?.type === 'MemberExpression' && !node.computed && node.property.type === 'Identifier'
    ? node.property.name
    : null
}

function memberName(node) {
  if (node?.type !== 'MemberExpression') return null
  if (node.computed && node.property.type === 'StringLiteral') return node.property.value
  return !node.computed && node.property.type === 'Identifier' ? node.property.name : null
}

export function resourceBoundaryViolations(filename, source) {
  const file = parse(source, { sourceType: 'unambiguous', plugins: ['typescript', 'jsx'] })
  const effectNames = new Set()
  const readNames = new Set(['fetch'])
  const readNamespaces = new Map()
  const routeResourceOwners = new Map()
  const functions = new Map()

  for (const node of file.program.body) {
    if (node.type !== 'ImportDeclaration' || node.importKind === 'type') continue
    const module = node.source.value
    const api = isApiModule(module)
    const route = !api && isRouteModule(module)
    const resourceOwner = isResourceModule(module)
    for (const binding of node.specifiers) {
      if (binding.importKind === 'type') continue
      if (binding.type === 'ImportDefaultSpecifier' && api) readNames.add(binding.local.name)
      if (resourceOwner && binding.type === 'ImportDefaultSpecifier') routeResourceOwners.set(binding.local.name, { module, namespace: false })
      if (binding.type === 'ImportNamespaceSpecifier') {
        if (resourceOwner) routeResourceOwners.set(binding.local.name, { module, namespace: true })
        if (api) readNamespaces.set(binding.local.name, null)
        else if (route) readNamespaces.set(binding.local.name, LOADER)
      }
      if (binding.type === 'ImportSpecifier') {
        const imported = binding.imported.name ?? binding.imported.value
        if (resourceOwner) routeResourceOwners.set(binding.local.name, { module, namespace: false })
        if (module === 'react' && /^(useEffect|useLayoutEffect)$/.test(imported)) effectNames.add(binding.local.name)
        if (api || (route && LOADER.test(imported))) readNames.add(binding.local.name)
      }
    }
  }

  walk(file.program, (node) => {
    if (node.type === 'FunctionDeclaration' && node.id) functions.set(node.id.name, node)
    if (node.type !== 'VariableDeclarator' || node.id.type !== 'Identifier' || !node.init) return
    if (isFunction(node.init)) functions.set(node.id.name, node.init)
    if (node.init.type === 'CallExpression' && isFunction(node.init.arguments[0])) {
      functions.set(node.id.name, node.init.arguments[0])
    }
  })

  const reads = (node, visited = new Set()) => {
    let found = false
    walk(node, (child) => {
      if (found) return
      if (child.type === 'Identifier' && functions.has(child.name) && !visited.has(child.name)) {
        visited.add(child.name)
        found = reads(functions.get(child.name), visited)
      }
      if (child.type !== 'CallExpression') return
      const callee = child.callee
      if (callee.type === 'Identifier' && readNames.has(callee.name)) found = true
      const member = calledMember(callee)
      if (!member) return
      const owner = callee.object
      if (owner.type !== 'Identifier') return
      if (readNamespaces.has(owner.name)) {
        const allowed = readNamespaces.get(owner.name)
        if (allowed === null || allowed.test(member)) found = true
      }
      if (member === 'fetch' && /^(window|globalThis)$/.test(owner.name)) found = true
    })
    return found
  }

  const violations = []
  walk(file.program, (node) => {
    if (node.type !== 'CallExpression') return
    const member = memberName(node.callee)
    if (filename.startsWith('src/routes/') && RESOURCE_WRITE.test(member ?? '')) {
      let target = node.callee.object
      while (target?.type === 'MemberExpression') target = target.object
      if (target?.type === 'Identifier') {
        const owner = routeResourceOwners.get(target.name)
        if (owner && (owner.namespace || node.callee.object === target)) {
          violations.push(`${filename}:${node.loc.start.line}: route publishes ${target.name} directly; use the ${owner.module} owner function`)
        }
      }
    }
    const effect = node.callee.type === 'Identifier' && effectNames.has(node.callee.name)
      || /^(useEffect|useLayoutEffect)$/.test(calledMember(node.callee) ?? '')
    if (effect && reads(node.arguments[0])) {
      violations.push(`${filename}:${node.loc.start.line}: component effect owns a server read; use the existing resource/cache owner`)
    }
  })
  return violations
}
