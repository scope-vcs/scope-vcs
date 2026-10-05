import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RequestChecksSection } from '@/features/requests/request-checks-section'
import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import './styles.css'

type GitHubCheck = Extract<RequestCheckResponse, { provider: 'github' }>
const github = (name: string, status: GitHubCheck['status'], conclusion: GitHubCheck['conclusion'] = null): RequestCheckResponse => ({
  provider: 'github', name, status, conclusion,
  details_url: status ? 'https://github.com/octo/demo/actions/runs/1/job/2' : null,
})
const V = 'Validate selected components'
const S = `${V} / Server validation`
/** The checks of a GitHub-checked request part way through, as a maintainer reported them. */
const running = {
  request_id: 'req_1', head_oid: 'a'.repeat(40), state: 'started', message: null, can_approve: false,
  changes_github_workflows: false, private_request_on_public_github: true,
  github_push: { state: 'sent', branch: 'scope/requests/req_1', error: null },
  checks: [
    github('Check operations', 'in_progress'),
    github('Check repository policy', 'completed', 'success'),
    github('Plan selected components', 'completed', 'success'),
    github('Required PR checks', 'completed', 'success'),
    github(`${V} / CLI validation`, null),
    github(`${V} / Integration validation`, null),
    github(`${V} / Production validation gate`, 'completed', 'success'),
    github(`${V} / Runner base image`, 'completed', 'skipped'),
    github(`${S} / Backend validation`, 'completed', 'skipped'),
    github(`${S} / Media worker image`, 'completed', 'skipped'),
    github(`${S} / Server validation gate`, 'completed', 'success'),
    github(`${S} / Web validation`, 'completed', 'skipped'),
  ],
} as unknown as RequestChecksResponse
/** A repository on Scope's own runner, whose checks have runs to open. */
const native = {
  ...running, private_request_on_public_github: false, github_push: null,
  checks: [
    { provider: 'native', workflow_path: '/.scope/runs/checks.yml', workflow_name: 'checks', run_id: 'run_checks', run_state: 'failed' },
    { provider: 'native', workflow_path: '/.scope/runs/lint.yml', workflow_name: 'lint', run_id: 'run_lint', run_state: 'succeeded' },
  ],
} as RequestChecksResponse

function Request() {
  const { owner, repo } = useParams({ strict: false })
  const [checks, setChecks] = useState(running)
  Object.assign(window, {
    showNative: () => setChecks(native),
    /** A live refresh: the same checks, the first one now finished. */
    refresh: () => setChecks((current) => ({
      ...current,
      checks: current.checks.map((check, index) => index ? check : github('Check operations', 'completed', 'success')),
    })),
  })
  return <main className="mx-auto max-w-3xl"><RequestChecksSection checks={checks} error={null} params={{ owner: owner!, repo: repo! }} /></main>
}

function Run() {
  const { runId } = useParams({ strict: false })
  return <h1>Run {runId}</h1>
}

const root = createRootRoute({ component: Outlet })
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: '/', component: () => <a href="/octo/demo/requests/req_1">Open request</a> }),
  createRoute({ getParentRoute: () => root, path: '$owner/$repo/requests/$requestId', component: Request }),
  createRoute({ getParentRoute: () => root, path: '$owner/$repo/runs/$runId', component: Run }),
])
createRoot(document.getElementById('root')!).render(<RouterProvider router={createRouter({ routeTree })} />)
