import { readFile } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const workspaceFile = 'web/pnpm-workspace.yaml'
const expiryByPackage = new Map([['@pierre/theme', '2027-01-28']])

export function trustExclusions(workspaceYaml) {
  const lines = workspaceYaml.split('\n')
  const start = lines.findIndex((line) => line.startsWith('trustPolicyExclude:'))
  if (start === -1) return []
  const exclusions = []
  for (const line of lines.slice(start + 1)) {
    const entry = /^\s+-\s+["']?([^"'\s]+)["']?\s*$/.exec(line)
    if (!entry) break
    exclusions.push(entry[1])
  }
  if (exclusions.length === 0) throw new Error('trustPolicyExclude is not a block list of package@version entries')
  return exclusions
}

export function expiredTrustExclusions(workspaceYaml, today, expiries = expiryByPackage) {
  return trustExclusions(workspaceYaml).flatMap((exclusion) => {
    const expiry = expiries.get(exclusion.slice(0, exclusion.lastIndexOf('@')))
    return expiry && today >= expiry ? [{ exclusion, expiry }] : []
  })
}

async function main() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..')
  const workspaceYaml = await readFile(path.join(root, workspaceFile), 'utf8')
  const expired = expiredTrustExclusions(workspaceYaml, new Date().toISOString().slice(0, 10))
  if (expired.length > 0) {
    console.error(`Trust exclusion guardrail failed. Remove each entry from ${workspaceFile}, or get its approval renewed and move its date in check-trust-exclusions.mjs.`)
    for (const { exclusion, expiry } of expired) console.error(`  ${exclusion} expired on ${expiry}`)
    process.exitCode = 1
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href) {
  await main()
}
