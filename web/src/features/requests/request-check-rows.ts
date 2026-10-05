import type { RequestCheckResponse, RequestChecksResponse } from '@/api/types.generated'
import { githubRunResult } from '../runs/github-run-status'
import { type RunTone, runStatus } from '../runs/run-status'

/** One check as the request page lists it, with the Scope run that has its logs. */
export type RequestCheckRow = {
  key: string
  name: string
  /** The job's own name, without the workflows it is nested in. */
  leaf: string
  /** The workflows and jobs that contain it, outermost first. */
  parents: string[]
  /** A state in the runs vocabulary, so checks share the run status icons. */
  state: string
  tone: RunTone
  label: string
  runId: string | null
}

/** What the checks section leads with, then the checks that still need someone. */
export type RequestChecksSummary = {
  /** `state` is in the runs vocabulary, so the lead shares the run status icons. */
  lead: { state: 'failed' | 'running' | 'succeeded'; text: string } | null
  counts: string | null
  /** Why the checks could not start, or why the last attempt to start them failed. */
  startError: { text: string; failed: boolean } | null
  /** Failed, then running, then waiting checks. */
  attention: RequestCheckRow[]
  /** Every check, ordered so that jobs sit under the workflows they belong to. */
  all: RequestCheckRow[]
}

const ATTENTION_ORDER: Partial<Record<RunTone, number>> = { danger: 0, running: 1, waiting: 2 }

function requestCheckRow(check: RequestCheckResponse): RequestCheckRow {
  if (check.provider === 'native') {
    return row(
      `native:${check.workflow_path}`,
      check.workflow_name,
      // A recorded check without a run is waiting, not passing, and a
      // canceled run blocks merging like a failed one.
      check.run_state === 'canceled' ? 'failed' : check.run_state ?? 'pending',
      check.run_state ? runStatus(check.run_state).label : 'not started',
      check.run_id,
    )
  }
  const result = check.status
    ? githubRunResult(check.status, check.conclusion)
    : { state: 'pending', label: 'waiting' }
  return row(`github:${check.name}`, check.name, result.state, result.label, null)
}

export function requestChecksSummary(checks: RequestChecksResponse): RequestChecksSummary {
  const all = checks.checks
    .map(requestCheckRow)
    .sort(byWorkflowPath)
  const attention = all
    .filter((check) => check.tone in ATTENTION_ORDER)
    .sort((a, b) => ATTENTION_ORDER[a.tone]! - ATTENTION_ORDER[b.tone]!)
  const count = (tone: RunTone) => all.filter((check) => check.tone === tone).length
  const failed = count('danger')
  const left = count('running') + count('waiting')
  const passed = count('success')
  const skipped = count('inert')

  const push = checks.github_push
  const lead: RequestChecksSummary['lead'] = push?.state === 'failed'
    ? { state: 'failed', text: 'Checks couldn’t start' }
    : push?.state === 'sending'
      ? { state: 'running', text: 'Starting checks' }
      : failed
        ? { state: 'failed', text: `${failed} failed` }
        : left
          ? { state: 'running', text: `${left} of ${all.length - skipped} left` }
          : passed
            ? { state: 'succeeded', text: `All ${passed} passed` }
            : null
  const counts = [
    failed && left ? `${left} left` : null,
    lead && lead.state !== 'succeeded' && passed ? `${passed} passed` : null,
    skipped ? `${skipped} skipped` : null,
  ].filter(Boolean)
  const startError = push?.error && (push.state === 'failed' || push.state === 'sending')
    ? push.state === 'failed'
      ? { text: push.error, failed: true }
      : { text: `The last attempt failed: ${push.error}`, failed: false }
    : null

  return { lead, counts: counts.length ? counts.join(' · ') : null, startError, attention, all }
}

/** One line of the full checks list: a workflow heading, or a check under it. */
export type RequestCheckTreeLine =
  | { kind: 'group'; key: string; name: string; depth: number }
  | { kind: 'check'; row: RequestCheckRow; depth: number }

/**
 * Nests checks under the workflows they belong to. A heading is emitted once
 * for each workflow the previous check was not already inside.
 */
export function requestCheckTree(rows: RequestCheckRow[]): RequestCheckTreeLine[] {
  const lines: RequestCheckTreeLine[] = []
  let open: string[] = []
  for (const row of rows) {
    let shared = 0
    while (shared < open.length && shared < row.parents.length && open[shared] === row.parents[shared]) {
      shared += 1
    }
    for (let depth = shared; depth < row.parents.length; depth += 1) {
      const path = row.parents.slice(0, depth + 1).join(' / ')
      lines.push({ kind: 'group', key: `group:${path}`, name: row.parents[depth]!, depth })
    }
    open = row.parents
    lines.push({ kind: 'check', row, depth: row.parents.length })
  }
  return lines
}

/**
 * Orders checks by their workflow path, one name at a time, so the checks of
 * a workflow stay together even beside a workflow whose name differs only in
 * case or accents.
 */
function byWorkflowPath(a: RequestCheckRow, b: RequestCheckRow) {
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
  name: string,
  state: string,
  label: string,
  runId: string | null,
): RequestCheckRow {
  const parts = name.split(' / ')
  const leaf = parts.pop()!
  return { key, name, leaf, parents: parts, state, tone: runStatus(state).tone, label, runId }
}
