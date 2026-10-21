import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import { githubRunResult } from '../runs/github-run-status'
import { runJobPanelId } from '../runs/run-job-ids'
import { type RunTone, runStatus } from '../runs/run-status'

export type RequestCheckRow = {
  key: string
  provider: RequestCheckResponse['provider']
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

type RequestChecksSummary = {
  status: RequestChecksStatus | null
  counts: string | null
  startError: { text: string; failed: boolean } | null
  attention: RequestCheckRow[]
  all: RequestCheckRow[]
}

const ATTENTION_ORDER: Partial<Record<RunTone, number>> = { danger: 0, running: 1, waiting: 2 }

function requestCheckRow(check: RequestCheckResponse): RequestCheckRow {
  if (check.provider === 'native') {
    return row(
      `native:${check.workflow_path}`,
      'native',
      null,
      check.workflow_name,
      check.run_state === 'canceled' ? 'failed' : check.run_state ?? 'pending',
      runStatus(check.run_state ?? 'pending').label,
      check.run_id ? { id: check.run_id } : null,
    )
  }
  const result = check.status
    ? githubRunResult(check.status, check.conclusion)
    : { state: 'pending', label: 'waiting' }
  return row(
    `github:${check.name}`,
    'github',
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
    .filter((check) => check.tone in ATTENTION_ORDER)
    .sort((a, b) => ATTENTION_ORDER[a.tone]! - ATTENTION_ORDER[b.tone]!)
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

  return { status, counts: counts.length ? counts.join(' · ') : null, startError, attention, all }
}

type RequestCheckGroup =
  | { kind: 'workflow'; key: string; name: string; runId: string; jobs: RequestCheckRow[] }
  | { kind: 'native'; key: string; row: RequestCheckRow }
  | { kind: 'unassigned'; key: string; jobs: RequestCheckRow[] }

export function requestCheckGroups(rows: RequestCheckRow[]): RequestCheckGroup[] {
  const groups = new Map<string, RequestCheckGroup>()
  for (const row of rows) {
    if (row.provider === 'native') {
      groups.set(row.key, { kind: 'native', key: row.key, row })
      continue
    }
    const key = row.workflow ? `workflow:${row.workflow.id}` : 'unassigned'
    let group = groups.get(key)
    if (!group) {
      group = row.workflow
        ? { kind: 'workflow', key, name: row.workflow.name, runId: row.workflow.id, jobs: [] }
        : { kind: 'unassigned', key, jobs: [] }
      groups.set(key, group)
    }
    if (group.kind !== 'native') group.jobs.push(row)
  }
  return [...groups.values()]
}

function byNamePath(a: RequestCheckRow, b: RequestCheckRow) {
  const left = [...a.parents, a.leaf]
  const right = [...b.parents, b.leaf]
  for (let index = 0; index < Math.min(left.length, right.length); index += 1) {
    const [x, y] = [left[index]!, right[index]!]
    if (x !== y) return x.localeCompare(y) || (x < y ? -1 : 1)
  }
  return left.length - right.length
}

function row(
  key: string,
  provider: RequestCheckResponse['provider'],
  workflow: RequestCheckRow['workflow'],
  name: string,
  state: string,
  label: string,
  run: RequestCheckRow['run'],
): RequestCheckRow {
  const parts = name.split(' / ')
  const leaf = parts.pop()!
  const tone = runStatus(state).tone
  const reason = tone === 'danger' && label !== 'failed' ? label : null
  return { key, provider, workflow, name, leaf, parents: parts, state, tone, reason, run }
}
