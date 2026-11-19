import type {
  RequestCheckResponse,
  RequestChecksResponse,
  RequestGitHubPullRequestResponse,
  RequestReviewBotResponse,
  RequestReviewBotState,
} from '@/api/types.generated'
import { githubRunResult } from '../../runs/github-run-status'
import { runJobPanelId } from '../../runs/run-job-ids'
import { type RunTone, runStatus } from '../../runs/run-status'

export type RequestCheckRow = {
  key: string
  workflow: { id: string; name: string } | null
  name: string
  leaf: string
  parents: string[]
  state: string
  tone: RunTone
  reason: string | null
  run: { id: string; hash?: string } | null
}

type RequestChecksStatus = { tone: RunTone; text: string }

export type RequestChecksSummary = {
  status: RequestChecksStatus | null
  counts: string | null
  startError: { text: string; failed: boolean } | null
  attention: RequestCheckRow[]
  all: RequestCheckRow[]
  track: RequestCheckRow[]
}

const ATTENTION_ORDER: Partial<Record<RunTone, number>> = { danger: 0, running: 1, waiting: 2 }
const TRACK_ORDER: Record<RunTone, number> = { danger: 0, success: 1, running: 2, waiting: 3, inert: 4 }

function requestCheckRow(check: RequestCheckResponse): RequestCheckRow {
  const result = check.status
    ? githubRunResult(check.status, check.conclusion)
    : { state: 'pending', label: 'waiting' }
  return row(
    check.name,
    check.run ? { id: check.run.run_id, name: check.run.workflow_name } : null,
    check.name,
    result.state,
    result.label,
    check.run ? { id: check.run.run_id, hash: runJobPanelId(check.run.job_id) } : null,
  )
}

export function requestChecksSummary(checks: RequestChecksResponse): RequestChecksSummary {
  const all = checks.checks
    .map(requestCheckRow)
    .sort(byNamePath)
  const attention = all
    .flatMap((check) => {
      const rank = ATTENTION_ORDER[check.tone]
      return rank === undefined ? [] : [{ check, rank }]
    })
    .sort((a, b) => a.rank - b.rank)
    .map(({ check }) => check)
  const count = (tone: RunTone) => all.filter((check) => check.tone === tone).length
  const failed = count('danger')
  const left = count('running') + count('waiting')
  const passed = count('success')
  const skipped = count('inert')

  const part = (total: number, tone: RunTone, text: string): RequestChecksStatus[] => total ? [{ tone, text }] : []
  const tally = [
    ...part(failed, 'danger', `${failed} failed`),
    ...part(left, 'running', failed ? `${left} left` : `${left} of ${all.length - skipped} left`),
    ...part(passed, 'success', `${passed} passed`),
    ...part(skipped, 'inert', `${skipped} skipped`),
  ]

  const push = checks.github_push
  const status: RequestChecksStatus | null = checks.state !== 'started'
    ? null
    : push?.state === 'failed'
      ? { tone: 'danger', text: 'Couldn’t start' }
      : push?.state === 'sending'
        ? { tone: 'running', text: 'Starting' }
        : tally[0] ?? null
  const counts = tally.filter((part) => part !== status).map((part) => part.text)
  const startError = push?.error && (push.state === 'failed' || push.state === 'sending')
    ? push.state === 'failed'
      ? { text: push.error, failed: true }
      : { text: `The last attempt failed: ${push.error}`, failed: false }
    : null

  const track = all
    .map((check, index) => ({ check, index }))
    .sort((a, b) => TRACK_ORDER[a.check.tone] - TRACK_ORDER[b.check.tone] || a.index - b.index)
    .map(({ check }) => check)

  return { status, counts: counts.length ? counts.join(' · ') : null, startError, attention, all, track }
}

export function requestListedChecks({ attention, all }: Pick<RequestChecksSummary, 'attention' | 'all'>) {
  return [...attention, ...all.filter((check) => !attention.includes(check))]
}

export function requestFailedCheck({ attention }: Pick<RequestChecksSummary, 'attention'>) {
  const failed = attention.filter((check) => check.tone === 'danger')
  return failed[0] ? { check: failed[0], others: failed.length - 1 } : null
}

export type RequestPullRequestView = {
  text: string
  url: string | null
  note: { text: string; failed: boolean } | null
  outOfStep: boolean
}

export function requestPullRequestView(
  pullRequest: RequestGitHubPullRequestResponse | null,
): RequestPullRequestView | null {
  if (!pullRequest) return null
  const named = pullRequest.number === null ? null : `Pull request #${pullRequest.number} on GitHub`
  const url = named ? pullRequest.url : null
  const error = pullRequest.error
  switch (pullRequest.state) {
    case 'opening':
      return {
        text: 'Opening a pull request on GitHub',
        url: null,
        note: error ? { text: `The last attempt failed: ${error}`, failed: false } : null,
        outOfStep: true,
      }
    case 'open':
      return { text: named ?? 'Pull request on GitHub', url, note: null, outOfStep: false }
    case 'closed':
      return { text: `${named ?? 'Pull request on GitHub'} · closed`, url, note: null, outOfStep: false }
    case 'closed_on_github':
      return { text: `${named ?? 'Pull request on GitHub'} · closed on GitHub`, url, note: null, outOfStep: true }
    case 'merged':
      return { text: `${named ?? 'Pull request on GitHub'} · merged`, url, note: null, outOfStep: false }
    case 'failed':
      return {
        text: named ? `${named} is out of step` : 'Scope couldn’t open a pull request on GitHub',
        url,
        note: error ? { text: error, failed: true } : null,
        outOfStep: true,
      }
  }
}

const REVIEW_BOT_STATES = {
  not_started: { label: 'Not started', tone: 'inert' },
  working: { label: 'Reviewing', tone: 'running' },
  commented: { label: 'Finished with comments', tone: 'waiting' },
  clear: { label: 'Finished, nothing found', tone: 'success' },
  timed_out: { label: 'Didn’t start · advisory', tone: 'inert' },
} as const satisfies Record<RequestReviewBotState, { label: string; tone: RunTone }>

const REVIEW_URGENCY: RequestReviewBotState[] = ['working', 'commented', 'not_started', 'timed_out', 'clear']

export type RequestReviewBotRow = {
  login: string
  name: string
  required: boolean
  state: RequestReviewBotState
  label: string
  tone: RunTone
}

export function requestReviewBotRows(bots: RequestReviewBotResponse[]): RequestReviewBotRow[] {
  return bots.map((bot) => ({
    login: bot.login,
    name: bot.login.replace(/\[bot\]$/, ''),
    required: bot.required,
    state: bot.state,
    ...REVIEW_BOT_STATES[bot.state],
  }))
}

export function requestReviewSummary(bots: RequestReviewBotRow[]) {
  const [first] = [...bots].sort((a, b) => REVIEW_URGENCY.indexOf(a.state) - REVIEW_URGENCY.indexOf(b.state))
  if (!first) return null
  return { state: first.state, label: first.state === 'clear' ? 'Review clear' : first.label }
}

type RequestCheckGroup =
  | { kind: 'workflow'; key: string; name: string; runId: string; jobs: RequestCheckRow[] }
  | { kind: 'unassigned'; key: string; jobs: RequestCheckRow[] }

export function requestCheckGroups(rows: RequestCheckRow[]): RequestCheckGroup[] {
  const groups = new Map<string, RequestCheckGroup>()
  for (const row of rows) {
    const key = row.workflow ? `workflow:${row.workflow.id}` : 'unassigned'
    let group = groups.get(key)
    if (!group) {
      group = row.workflow
        ? { kind: 'workflow', key, name: row.workflow.name, runId: row.workflow.id, jobs: [] }
        : { kind: 'unassigned', key, jobs: [] }
      groups.set(key, group)
    }
    group.jobs.push(row)
  }
  return [...groups.values()]
}

function byNamePath(a: RequestCheckRow, b: RequestCheckRow) {
  const left = [...a.parents, a.leaf]
  const right = [...b.parents, b.leaf]
  for (const [index, x] of left.entries()) {
    const y = right[index]
    if (y === undefined) break
    if (x !== y) return x.localeCompare(y) || (x < y ? -1 : 1)
  }
  return left.length - right.length
}

function row(
  key: string,
  workflow: RequestCheckRow['workflow'],
  name: string,
  state: string,
  label: string,
  run: RequestCheckRow['run'],
): RequestCheckRow {
  const parts = name.split(' / ')
  const leaf = parts.pop() ?? name
  const tone = runStatus(state).tone
  const reason = tone === 'danger' && label !== 'failed' ? label : null
  return { key, workflow, name, leaf, parents: parts, state, tone, reason, run }
}
