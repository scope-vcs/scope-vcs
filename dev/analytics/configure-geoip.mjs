import { readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import { posthogRequest } from './sync-reports.mjs'

export async function configureGeoip(request, hog, { apply = false } = {}) {
  const project = await request('')
  const matches = []
  const visited = new Set()
  let next = 'hog_functions/?type=transformation&limit=1000'
  while (next) {
    if (visited.has(next)) throw new Error('PostHog returned a pagination cycle')
    visited.add(next)
    const page = await request(next)
    matches.push(...page.results.filter((item) => item.template?.id === 'template-geoip'))
    next = page.next
  }
  if (matches.length !== 1 || matches[0].deleted) {
    throw new Error('Expected exactly one existing default GeoIP transformation; no settings were changed')
  }
  const functionPath = `hog_functions/${encodeURIComponent(matches[0].id)}/`
  const existing = await request(functionPath)
  if (existing.draft != null || existing.draft_updated_at != null) {
    throw new Error('GeoIP transformation has a pending draft; resolve it before configuring')
  }
  const name = 'Scope approximate location'
  const updateProject = project.anonymize_ips !== true
  const updateFunction = existing.hog !== hog || existing.name !== name || existing.enabled !== true
  if (updateFunction && (!existing.updated_at || !Number.isFinite(Date.parse(existing.updated_at))
    || !Number.isFinite(existing.execution_order))) {
    throw new Error('GeoIP transformation is missing its concurrency timestamp or execution order; no settings were changed')
  }
  const result = {
    projectId: project.id,
    functionId: existing.id,
    anonymizeIps: updateProject ? 'update-required' : 'current',
    transformation: updateFunction ? 'update-required' : 'current',
    status: 'preview',
  }
  if (!apply) return result
  if (updateProject) await request('', { method: 'PATCH', body: { anonymize_ips: true } })
  if (updateFunction) {
    await request(functionPath, { method: 'PATCH', body: {
      hog, name, enabled: true, execution_order: existing.execution_order, base_updated_at: existing.updated_at,
    } })
  }
  const verifiedProject = await request('')
  const verifiedFunction = await request(functionPath)
  if (verifiedProject.anonymize_ips !== true || verifiedFunction.hog !== hog
    || verifiedFunction.enabled !== true || verifiedFunction.name !== name
    || verifiedFunction.draft != null || verifiedFunction.draft_updated_at != null
    || verifiedFunction.execution_order !== existing.execution_order || verifiedFunction.deleted) {
    throw new Error('GeoIP configuration verification failed; inspect project settings and the live transformation before retrying')
  }
  return { ...result, anonymizeIps: 'current', transformation: 'current', status: 'verified' }
}

async function main() {
  const args = process.argv.slice(2)
  if (args.length !== 1 || !['--preview', '--apply'].includes(args[0])) {
    throw new Error('Usage: node dev/analytics/configure-geoip.mjs --preview|--apply')
  }
  const hog = await readFile(new URL('./geoip.hog', import.meta.url), 'utf8')
  const result = await configureGeoip(posthogRequest({
    apiKey: process.env.POSTHOG_PERSONAL_API_KEY, projectId: process.env.POSTHOG_PROJECT_ID,
    host: process.env.POSTHOG_APP_HOST,
  }), hog, { apply: args[0] === '--apply' })
  console.log(JSON.stringify(result, null, 2))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1 })
}
