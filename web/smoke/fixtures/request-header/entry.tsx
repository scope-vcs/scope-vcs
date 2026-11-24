import { createRoot } from 'react-dom/client'
import { createRootRoute, createRouter, RouterProvider } from '@tanstack/react-router'
import { useRef, useState } from 'react'
import { ChevronDown, SlidersHorizontal } from 'lucide-react'
import { Button } from '@/components/ui/button'
import type { RequestAuthorRecordResponse, RequestChecksResponse, RequestDetailResponse, RequestSummaryResponse } from '@/api/types.generated'
import { RequestChecksSection } from '@/features/requests/checks/request-checks-section'
import { RequestChangeCommand } from '@/features/requests/detail/request-change-command'
import { RequestDetailHeader } from '@/features/requests/detail/request-detail-header'
import { RequestChangesMenu } from '@/features/requests/changes/request-changes-menu'
import { RequestLifecycleActions } from '@/features/requests/detail/request-lifecycle-actions'
import { RequestMoreMenu } from '@/features/requests/detail/request-more-menu'
import '@/features/requests/detail/request-detail.css'
import '../shared/styles.css'

const params = { owner: 'adamblumoff', repo: 'scope-vcs' }
const requestParams = { ...params, requestId: 'request' }
const request = {
  id: 'request', name: 'repository-row-security', title: 'Enforce repository access',
  head_oid: 'a'.repeat(40), view: 'private', state: 'Open', author_role: 'Member',
  base: { kind: 'main', review_base_oid: 'b'.repeat(40) }, base_version: 1,
  stack_status: 'main', stack_depth: 1, submitted_at_unix: 1793707200, merged_at_unix: null, closed_at_unix: null,
  mergeability: { status: 'ReviewFindingsOpen', reason: 'Review findings remain open.' },
  permissions: { can_merge: true, can_close: true, can_submit: false, can_view_activity: true, can_read_change: true },
} as RequestSummaryResponse
const asked = {
  ...request, title: 'Login times out after 30 seconds on slow connections', state: 'Draft',
  view: 'public', submitted_at_unix: null, author_user_id: null,
  ask: {
    handle: 'gh-142', github_issue_number: 142, source_url: 'https://github.com/adamblumoff/scope-vcs/issues/142',
    text_markdown: 'Sign-in gives up before the server answers.', asked_at_unix: 1786000000,
    suggested_branch_name: 'login-times-out-after-30-seconds',
  },
  mergeability: { status: 'NoChangeYet', reason: 'request has no change yet' },
  permissions: { can_close: true, can_view_activity: true, can_start_work: true, can_attach_change: true },
} as RequestSummaryResponse
const check = (conclusion: 'success' | 'failure', name = 'Required PR checks') => ({
  name, status: 'completed', conclusion, details_url: null,
  run: { run_id: '9002', workflow_name: 'PR requirements', job_id: '45' },
} as const)
const bots = (codex: 'commented' | 'not_started', greptile: 'clear' | 'not_started') => [
  { login: 'chatgpt-codex-connector[bot]', required: true, state: codex },
  { login: 'greptile-apps[bot]', required: false, state: greptile },
]
const baseChecks: RequestChecksResponse = {
  request_id: request.id, head_oid: request.head_oid, state: 'started', message: null, can_approve: false,
  changes_github_workflows: false, private_request_on_public_github: true,
  github_push: { state: 'sent', branch: 'scope/requests/request', error: null },
  github_pull_request: { state: 'open', number: 1108, url: 'https://github.com/adamblumoff/scope-vcs/pull/1108', error: null },
  review_bots: bots('commented', 'clear'), review_bots_enforced: true, checks: [check('success')],
}
const failed = {
  request: { ...request, mergeability: { status: 'ChecksFailed', reason: 'Required CI failed.' } },
  checks: { ...baseChecks, checks: [check('failure')], review_bots: bots('not_started', 'not_started') },
  canEnable: false,
} satisfies { request: RequestSummaryResponse; checks: RequestChecksResponse; canEnable: boolean }
const STATES = {
  findings: { request, checks: baseChecks, canEnable: true },
  failed,
  longFailure: {
    ...failed,
    checks: { ...failed.checks, checks: [check('failure', `required_${'web_validation_'.repeat(10)}`)] },
  },
  merged: {
    request: {
      ...request, state: 'Merged', merged_at_unix: 1793880000, mergeability: { status: 'Merged', reason: null },
      permissions: { ...request.permissions, can_merge: false, can_close: false },
    },
    checks: { ...baseChecks, review_bots: bots('not_started', 'not_started') },
    canEnable: false,
  },
  asked: { request: asked, checks: baseChecks, canEnable: false },
  working: {
    request: {
      ...asked, author_user_id: 'author',
      permissions: { ...asked.permissions, can_start_work: false, can_release_work: true },
    },
    checks: baseChecks,
    canEnable: false,
  },
  declined: {
    request: {
      ...asked, state: 'Closed', closed_at_unix: 1793880000, close_reason: 'not_planned',
      mergeability: { status: 'Closed', reason: null }, permissions: { can_view_activity: true },
    },
    checks: baseChecks,
    canEnable: false,
  },
} satisfies Record<string, { request: RequestSummaryResponse; checks: RequestChecksResponse; canEnable: boolean }>
const activity = { identity: 'header-activity', version: '1', load: async () => ({ events: [], next_cursor: null }) }
const actions = { error: null, pending: null, run: async () => false }
const autoMerge = {
  error: null, pending: null, authorize: async () => true, cancel: async () => true,
  status: {
    can_cancel: false, can_enable: true, head_oid: request.head_oid, intent: null,
    request_id: request.id, revision_id: 'revision', waiting_reason: request.mergeability.reason,
  },
}

function App() {
  const [name, setName] = useState(request.name)
  const [stateName, setStateName] = useState<keyof typeof STATES>('findings')
  const [authorRecord, setAuthorRecord] = useState<RequestAuthorRecordResponse | null>(null)
  const more = useRef<HTMLButtonElement | null>(null)
  const shown = STATES[stateName]
  const { ask, permissions } = shown.request
  Object.assign(window, { setBranch: setName, setState: setStateName, setAuthorRecord })
  return <main>
    <RequestDetailHeader
      checks={shown.checks} params={params}
      detail={{
        request: { ...shown.request, name, branch_name: ask ? null : name },
        author: ask && !shown.request.author_user_id ? null : { id: 'author', handle: 'adamblumoff' },
        asker: ask ? { github_user_id: 7, login: 'tomasz-k', account_type: 'user', avatar_url: null } : null,
        author_record: authorRecord, claimer: null,
      } as RequestDetailResponse}
      backLink={<a href="#requests">Requests</a>}
      stateActions={
        <RequestLifecycleActions actions={actions} autoMerge={{ ...autoMerge, status: { ...autoMerge.status, can_enable: shown.canEnable } }} request={shown.request} viewerId="viewer"
          className="fixed inset-x-0 bottom-0 z-30 justify-end border-t border-border bg-background px-(--app-inset) py-3 min-[701px]:static min-[701px]:border-0 min-[701px]:p-0" />
      }
      actions={<>
        {permissions.can_read_change ? <RequestChangesMenu activity={activity} diffStat={{ additions: 5495, deletions: 2578, changed_files: 203 }} params={requestParams} /> : null}
        <Button size="sm" variant="secondary"><SlidersHorizontal />Details<ChevronDown /></Button>
        <RequestMoreMenu actions={actions} disabled={false} onViewActivity={() => {}} params={params} request={shown.request} triggerRef={more} />
      </>}
    />
    {ask && permissions.can_attach_change ? <RequestChangeCommand ask={ask} /> : null}
    {permissions.can_read_change ? <RequestChecksSection checks={shown.checks} error={null} params={params} /> : null}
  </main>
}

const root = createRootRoute({ component: App })
createRoot(document.getElementById('root')!).render(<RouterProvider router={createRouter({ routeTree: root })} />)
