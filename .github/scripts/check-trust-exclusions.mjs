import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const workspaceFile = 'web/pnpm-workspace.yaml'
const expiryByPackage = new Map([['@pierre/theme', '2027-01-28']])
const advisoryExceptions = new Map([
  ['GHSA-fc5q-39xc-vjpf', {
    expiry: '2026-12-09',
    approver: 'Adam Blumoff',
    reason: 'picomatch ReDoS with no fixed release, reached only by build tooling',
  }],
])

function blockList(lines, start, name) {
  const indent = lines[start].length - lines[start].trimStart().length
  const entries = []
  for (const line of lines.slice(start + 1)) {
    if (line.trim() === '' || line.trimStart().startsWith('#')) continue
    if (line.length - line.trimStart().length <= indent) break
    const entry = /^\s+-\s+["']?([^"'\s]+)["']?\s*$/.exec(line)
    if (!entry) throw new Error(`${name} has an entry the check cannot read: ${line.trim()}`)
    entries.push(entry[1])
  }
  if (entries.length === 0) throw new Error(`${name} is not a block list of entries`)
  return entries
}

export function trustExclusions(workspaceYaml) {
  const lines = workspaceYaml.split('\n')
  const start = lines.findIndex((line) => line.startsWith('trustPolicyExclude:'))
  return start === -1 ? [] : blockList(lines, start, 'trustPolicyExclude')
}

function ignoredAdvisories(workspaceYaml) {
  const lines = workspaceYaml.split('\n')
  const start = lines.findIndex((line) => /^\s+ignoreGhsas:/.test(line))
  if (start !== -1) return blockList(lines, start, 'ignoreGhsas')
  if (workspaceYaml.includes('ignoreGhsas')) throw new Error('ignoreGhsas is not a block list of entries')
  return []
}

export function expiredTrustExclusions(workspaceYaml, today, expiries = expiryByPackage) {
  return trustExclusions(workspaceYaml).flatMap((exclusion) => {
    const expiry = expiries.get(exclusion.slice(0, exclusion.lastIndexOf('@')))
    return expiry && today >= expiry ? [{ exclusion, expiry }] : []
  })
}

export function lapsedAdvisoryExceptions(workspaceYaml, today, exceptions = advisoryExceptions) {
  return ignoredAdvisories(workspaceYaml).flatMap((advisory) => {
    const exception = exceptions.get(advisory)
    if (!exception?.expiry || !exception.approver || !exception.reason) {
      return [`${advisory} has no recorded reason, expiry, and approver`]
    }
    return today >= exception.expiry
      ? [`${advisory} (${exception.reason}; approved by ${exception.approver}) expired on ${exception.expiry}`]
      : []
  })
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const workspaceYaml = await readFile(path.join(root, workspaceFile), 'utf8')
  const today = new Date().toISOString().slice(0, 10)
  const expired = expiredTrustExclusions(workspaceYaml, today)
  if (expired.length > 0) {
    console.error(`Trust exclusion guardrail failed. Remove each entry from ${workspaceFile}, or get its approval renewed and move its date in check-trust-exclusions.mjs.`)
    for (const { exclusion, expiry } of expired) console.error(`  ${exclusion} expired on ${expiry}`)
    process.exitCode = 1
  }
  const lapsed = lapsedAdvisoryExceptions(workspaceYaml, today)
  if (lapsed.length > 0) {
    console.error(`Advisory exception guardrail failed. Remove each ignored advisory from ${workspaceFile}, or record a named approver's renewal in check-trust-exclusions.mjs.`)
    for (const failure of lapsed) console.error(`  ${failure}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
