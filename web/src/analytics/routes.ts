export type AnalyticsRoute = {
  name: string
  path: string
}

export type AnalyticsRouteDecision =
  | { kind: 'excluded' }
  | { kind: 'tracked'; route: AnalyticsRoute }

const excluded = { kind: 'excluded' } as const
const tracked = (name: string, path: string): AnalyticsRouteDecision => ({
  kind: 'tracked',
  route: { name, path },
})

// Requiring every generated route ID here turns a new page into a deliberate
// analytics decision. Layout routes are excluded because their leaf page owns
// the pageview.
const routeDecisions = {
  '__root__': excluded,
  '/': tracked('home', '/'),
  '/account': tracked('account', '/account'),
  '/cli-login': tracked('cli_login', '/cli-login'),
  '/licenses': tracked('licenses', '/licenses'),
  '/invites/$token': tracked('invite', '/invite'),
  '/sign-in/$': tracked('sign_in', '/sign-in'),
  '/sign-up/$': tracked('sign_up', '/sign-up'),
  '/$owner': excluded,
  '/$owner/': tracked('owner', '/owner'),
  '/$owner/$repo': excluded,
  '/$owner/$repo/_code': excluded,
  '/$owner/$repo/_code/': tracked('repository_code', '/repository/code'),
  '/$owner/$repo/history': tracked('repository_history', '/repository/history'),
  '/$owner/$repo/requests': excluded,
  '/$owner/$repo/requests/': tracked(
    'repository_requests',
    '/repository/requests',
  ),
  '/$owner/$repo/requests/$requestId': excluded,
  '/$owner/$repo/requests/$requestId/': tracked(
    'request',
    '/repository/request',
  ),
  '/$owner/$repo/requests/$requestId/changes': tracked(
    'request_changes',
    '/repository/request/changes',
  ),
  '/$owner/$repo/requests/$requestId/details': tracked(
    'request_details',
    '/repository/request/details',
  ),
  '/$owner/$repo/runs': excluded,
  '/$owner/$repo/runs/': tracked('repository_runs', '/repository/runs'),
  '/$owner/$repo/runs/$runId': tracked('repository_run', '/repository/run'),
  '/$owner/$repo/runs/workflows/$workflow': tracked(
    'repository_workflow',
    '/repository/workflow',
  ),
  '/$owner/$repo/settings': tracked(
    'repository_settings',
    '/repository/settings',
  ),
} satisfies Readonly<Record<string, AnalyticsRouteDecision>>

const routesByName = new Map(
  Object.values(routeDecisions).flatMap((decision) => (
    decision.kind === 'tracked'
      ? [[decision.route.name, decision.route] as const]
      : []
  )),
)

export function analyticsRouteDecisionForId(routeId: string | undefined) {
  if (!routeId || !(routeId in routeDecisions)) return null
  return routeDecisions[routeId as keyof typeof routeDecisions]
}

export function analyticsRouteIds() {
  return Object.keys(routeDecisions)
}

export function analyticsRouteForId(routeId: string | undefined) {
  const decision = analyticsRouteDecisionForId(routeId)
  return decision?.kind === 'tracked' ? decision.route : null
}

export function analyticsRouteForName(name: string) {
  return routesByName.get(name) ?? null
}
