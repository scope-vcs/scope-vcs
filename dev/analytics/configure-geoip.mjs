import { createHash } from 'node:crypto'
import { readFile, writeFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import { listAll, posthogRequest } from './sync-reports.mjs'

export async function configureGeoip(request, hog, { apply = false } = {}) {
  hog = hog.trim()
  const project = await request('')
  const matches = (await listAll(request, 'hog_functions/?type=transformation&limit=1000'))
    .filter((item) => item.template?.id === 'template-geoip')
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
  if (args.length !== 1 || !['--preview', '--apply', '--export-bytecode'].includes(args[0])) {
    throw new Error('Usage: node dev/analytics/configure-geoip.mjs --preview|--apply|--export-bytecode')
  }
  const hog = await readFile(new URL('./geoip.hog', import.meta.url), 'utf8')
  const request = posthogRequest({
    apiKey: process.env.POSTHOG_PERSONAL_API_KEY, projectId: process.env.POSTHOG_PROJECT_ID,
    host: process.env.POSTHOG_APP_HOST,
  })
  const result = await configureGeoip(request, hog, { apply: args[0] === '--apply' })
  if (args[0] === '--export-bytecode') {
    const compiled = await request(`hog_functions/${encodeURIComponent(result.functionId)}/`)
    if (compiled.hog !== hog.trim() || !Array.isArray(compiled.bytecode) || compiled.draft != null) {
      throw new Error('Configure the test project with this exact source before exporting bytecode')
    }
    await writeFile(new URL('./geoip.bytecode.json', import.meta.url), JSON.stringify({
      sourceSha256: createHash('sha256').update(hog).digest('hex'),
      bytecode: compiled.bytecode,
    }, null, 2) + '\n')
    result.status = 'bytecode-exported'
  }
  console.log(JSON.stringify(result, null, 2))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1 })
}
