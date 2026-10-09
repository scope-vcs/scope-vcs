import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { createRootRoute, createRoute, createRouter, Outlet, RouterProvider, useParams } from '@tanstack/react-router'
import { RequestCiApproval } from '@/features/requests/request-ci-approval'
import { RequestChecksSection } from '@/features/requests/request-checks-section'
import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import './styles.css'

type GitHubCheck = Extract<RequestCheckResponse, { provider: 'github' }>
const github = (name: string, status: GitHubCheck['status'], conclusion: GitHubCheck['conclusion'] = null, run: GitHubCheck['run'] = null): RequestCheckResponse => ({
  provider: 'github', name, status, conclusion, run,
  details_url: status ? 'https://github.com/octo/demo/actions/runs/1/job/2' : null,
})
const V = 'Validate selected components'
const S = `${V} / Server validation`
const running = {
  request_id: 'req_1', head_oid: 'a'.repeat(40), state: 'started', message: null, can_approve: false,
  changes_github_workflows: false, private_request_on_public_github: true,
  github_push: { state: 'sent', branch: 'scope/requests/req_1', error: null },
  checks: [
    github('Check operations', 'in_progress', null, { run_id: '9001', job_id: '31' }),
    github('Check repository policy', 'completed', 'success'),
    github('Plan selected components', 'completed', 'success'),
    github('Required PR checks', 'completed', 'success'),
    github(`${V} / CLI validation`, 'queued', null, { run_id: '9002', job_id: '45' }),
    github(`${V} / Integration validation`, null),
    github(`${V} / Production validation gate`, 'completed', 'success'),
    github(`${V} / Runner base image`, 'completed', 'skipped'),
    github(`${S} / Backend validation`, 'completed', 'skipped'),
    github(`${S} / Media worker image`, 'completed', 'skipped'),
    github(`${S} / Server validation gate`, 'completed', 'success'),
    github(`${S} / Web validation`, 'completed', 'skipped'),
  ],
} as unknown as RequestChecksResponse
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
    showState: (state: RequestChecksResponse['state']) => setChecks({
      ...running, state, checks: [], github_push: null, private_request_on_public_github: false,
    }),
    showApproval: () => setChecks({
      ...running, state: 'awaiting-approval', can_approve: true, changes_github_workflows: true,
    }),
    pushRevision: () => setChecks((current) => ({ ...current, head_oid: 'b'.repeat(40) })),
    refresh: () => setChecks((current) => ({
      ...current,
      checks: current.checks.map((check, index) => index ? check : { ...check as GitHubCheck, status: 'completed', conclusion: 'success' }),
    })),
  })
  return (
    <main className="mx-auto max-w-3xl">
      <RequestCiApproval
        controller={{
          checks, approving: false, error: null,
          approve: async (head) => {
            Object.assign(window, { approvedHead: head })
            return true
          },
        }}
        requestViewName="Agent"
      />
      <RequestChecksSection checks={checks} error={null} params={{ owner: owner!, repo: repo! }} requestViewName="Agent" />
    </main>
  )
}

function Run() {
  const { runId } = useParams({ strict: false })
  return <h1>Run {runId}{location.hash}</h1>
}

const root = createRootRoute({ component: Outlet })
const routeTree = root.addChildren([
  createRoute({ getParentRoute: () => root, path: '/', component: () => <a href="/octo/demo/requests/req_1">Open request</a> }),
  createRoute({ getParentRoute: () => root, path: '$owner/$repo/requests/$requestId', component: Request }),
  createRoute({ getParentRoute: () => root, path: '$owner/$repo/runs/$runId', component: Run }),
])
createRoot(document.getElementById('root')!).render(<RouterProvider router={createRouter({ routeTree })} />)
