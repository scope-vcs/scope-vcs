import { pathToFileURL } from 'node:url'
import { buildProductReports } from './reports.mjs'

export async function syncProductReports(request, reports) {
  // Validate every query before making any dashboard/insight changes.
  for (const report of reports) {
    const validation = await request('query/', { method: 'POST', body: {
      query: report.query.source, name: `Validate ${report.name}`, refresh: 'blocking',
    } })
    if (!Array.isArray(validation.results) || validation.error) {
      throw new Error('PostHog did not finish query validation; no reports were changed')
    }
  }
  const dashboardName = 'Scope product outcomes'
  const dashboards = await listAll('dashboards/')
  const matches = dashboards.filter((item) => !item.deleted && item.name === dashboardName)
  if (matches.length > 1) throw new Error('Multiple Scope product outcomes dashboards; resolve the duplicate before syncing')
  const dashboard = matches[0] ?? await request('dashboards/', { method: 'POST', body: {
    name: dashboardName,
    description: 'Production product outcomes. Definitions maintained in dev/analytics/reports.mjs. Counts describe captured events; delivery is best effort.',
  } })
  const insights = await listAll(`insights/?${new URLSearchParams({ dashboards: JSON.stringify([dashboard.id]) })}`)
  const results = []
  for (const report of reports) {
    const tag = `scope-product-report:${report.key}`
    const matches = insights.filter((item) => !item.deleted && item.tags?.includes(tag))
    if (matches.length > 1) throw new Error(`Multiple insights tagged ${tag}; resolve the duplicate before syncing`)
    const existing = matches[0]
    const payload = {
      name: report.name, description: report.description, query: report.query,
      tags: [...new Set([...(existing?.tags ?? []), tag])],
      dashboards: [...new Set([...(existing?.dashboards ?? []), dashboard.id])],
    }
    const saved = await request(existing ? `insights/${existing.id}/` : 'insights/', {
      method: existing ? 'PATCH' : 'POST', body: payload,
    })
    results.push({ key: report.key, id: saved.id, shortId: saved.short_id })
  }
  return { dashboardId: dashboard.id, insights: results }

  async function listAll(path) {
    const items = []
    let next = path
    const visited = new Set()
    while (next) {
      if (visited.has(next)) throw new Error('PostHog returned a pagination cycle')
      visited.add(next)
      const page = await request(next)
      items.push(...page.results)
      next = page.next
    }
    return items
  }
}

export function posthogRequest({ apiKey, projectId, host = 'https://us.posthog.com', fetchImpl = fetch }) {
  if (!/^\d+$/.test(projectId ?? '')) throw new Error('POSTHOG_PROJECT_ID must be numeric')
  if (!apiKey) throw new Error('POSTHOG_PERSONAL_API_KEY is required')
  if (!['https://us.posthog.com', 'https://eu.posthog.com'].includes(host)) {
    throw new Error('POSTHOG_APP_HOST must be the US or EU PostHog app host')
  }
  const base = `${host}/api/projects/${projectId}/`
  return async (path, { method = 'GET', body } = {}) => {
    const url = new URL(path, base)
    // Pagination cannot forward credentials to a different host or project.
    if (!url.href.startsWith(base) || url.username || url.password) throw new Error('Unexpected PostHog API URL')
    const response = await fetchImpl(url, {
      method, redirect: 'error', signal: AbortSignal.timeout(60_000),
      headers: { authorization: `Bearer ${apiKey}`, 'content-type': 'application/json' },
      ...(body ? { body: JSON.stringify(body) } : {}),
    })
    if (!response.ok) throw new Error(`PostHog ${method} ${url.pathname} failed with HTTP ${response.status}`)
    const result = await response.json()
    return result
  }
}

async function main() {
  const args = process.argv.slice(2)
  if (args.length !== 1 || !['--preview', '--apply'].includes(args[0])) {
    throw new Error('Usage: node dev/analytics/sync-reports.mjs --preview|--apply')
  }
  const csv = (name) => (process.env[name] ?? '').split(',').map((id) => id.trim()).filter(Boolean)
  const reports = buildProductReports({
    excludedUserIds: csv('SCOPE_ANALYTICS_EXCLUDED_USER_IDS'),
    excludedRepositoryIds: csv('SCOPE_ANALYTICS_EXCLUDED_REPOSITORY_IDS'),
  })
  if (args[0] === '--preview') return console.log(JSON.stringify(reports, null, 2))
  const result = await syncProductReports(posthogRequest({
    apiKey: process.env.POSTHOG_PERSONAL_API_KEY, projectId: process.env.POSTHOG_PROJECT_ID,
    host: process.env.POSTHOG_APP_HOST,
  }), reports)
  console.log(JSON.stringify(result, null, 2))
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => { console.error(error.message); process.exitCode = 1 })
}
