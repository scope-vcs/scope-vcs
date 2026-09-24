import assert from 'node:assert/strict'
import { existsSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { parse } from '@babel/parser'

const serverBundle = process.env.SCOPE_SMOKE_SERVER_MANIFEST
  ? pathToFileURL(resolve(process.env.SCOPE_SMOKE_SERVER_MANIFEST))
  : new URL('../.output/server/_ssr/ssr.mjs', import.meta.url)

// The emitted TanStack manifest owns production IDs. Staging supplies the
// deployed web build's manifest through SCOPE_SMOKE_SERVER_MANIFEST.
export function builtServerFunctions() {
  if (!existsSync(serverBundle)) return new Map()
  const source = parse(readFileSync(serverBundle, 'utf8'), { sourceType: 'module' })
  const functions = new Map()
  const visit = (node) => {
    if (!node || typeof node !== 'object') return
    if (Array.isArray(node)) {
      for (const child of node) visit(child)
      return
    }
    if (typeof node.type !== 'string') return
    if (node.type === 'ObjectProperty' && node.key.type === 'StringLiteral' && node.value.type === 'ObjectExpression') {
      const name = node.value.properties.find((property) =>
        property.type === 'ObjectProperty' && property.key.name === 'functionName')
      if (name?.value.type === 'StringLiteral') functions.set(node.key.value, name.value.value)
    }
    for (const [key, value] of Object.entries(node)) {
      if (key !== 'loc' && key !== 'extra' && key !== 'comments') visit(value)
    }
  }
  visit(source.program)
  return functions
}

let productionFunctions

export function serverFunctionName(request) {
  const pathname = new URL(request.url()).pathname
  if (!pathname.startsWith('/_serverFn/')) return ''
  const id = pathname.slice('/_serverFn/'.length)
  const productionName = /^[0-9a-f]{64}$/.test(id)
    ? (productionFunctions ??= builtServerFunctions()).get(id)
    : undefined
  if (productionName) return productionName

  let developmentName
  try {
    developmentName = JSON.parse(Buffer.from(id, 'base64url').toString('utf8')).export
  } catch {}
  assert(typeof developmentName === 'string' && developmentName.length > 0,
    `Unknown server function ID ${JSON.stringify(id)}; check the compiled server-function manifest`)
  return developmentName
}
