import assert from 'node:assert/strict'
import { describe, it } from 'node:test'
import type { RepositoryRunAttemptResponse, RepositoryRunDetailResponse } from '@/api/types.generated'
import { runJobLinkForHash, runLogLineId } from './run-job-ids'
import { readStepLogLines } from './run-log-lines'
import {
  attemptForJob,
  jobsHaveDependencies,
  latestAttempt,
  mergeStepLogPage,
  mergeStepLogs,
  openingView,
  reconcileAttemptOverrides,
  runCanChange,
  selectAttempt,
  selectJob,
  selectStep,
} from './repository-run-detail-model'

function job(overrides: {
  attempts?: Array<{
    id: string
    number: number
    steps: Array<{ index: number; state: string }>
  }>
  key: string
  needs?: string[]
  state: string
}) {
  return {
    attempts: overrides.attempts ?? [],
    job: { key: overrides.key, needs: overrides.needs ?? [], state: overrides.state },
  }
}

describe('repository run detail model', () => {
  it('identifies states that can still change', () => {
    for (const state of ['queued', 'dispatching', 'running'] as const) {
      assert.equal(runCanChange(state), true)
    }
    for (const state of ['succeeded', 'failed', 'canceled', 'lost'] as const) {
      assert.equal(runCanChange(state), false)
    }
  })

  it('picks the selected step attempt over the switcher and default attempt', () => {
    const target = job({
      attempts: [
        { id: 'a1', number: 1, steps: [{ index: 0, state: 'failed' }] },
        { id: 'a2', number: 2, steps: [{ index: 0, state: 'succeeded' }] },
      ],
      key: 'backend',
      state: 'succeeded',
    })
    assert.equal(attemptForJob(target, {}, null)?.id, 'a2')
    assert.equal(attemptForJob(target, { backend: 'a1' }, null)?.id, 'a1')
    assert.equal(
      attemptForJob(target, { backend: 'a1' }, {
        attemptId: 'a2',
        jobKey: 'backend',
        stepIndex: 0,
      })?.id,
      'a2',
    )
  })

  it('drops attempt overrides that reference retired attempts', () => {
    const jobs = [
      job({ attempts: [{ id: 'new', number: 1, steps: [] }], key: 'backend', state: 'succeeded' }),
    ]
    assert.deepEqual(
      reconcileAttemptOverrides({ backend: 'old', missing: 'x' }, jobs),
      {},
    )
    assert.deepEqual(
      reconcileAttemptOverrides({ backend: 'new' }, jobs),
      { backend: 'new' },
    )
  })

  it('offers the graph only when some job waits on another', () => {
    const independent = [0, 1, 2, 3].map((index) =>
      job({ key: `job-${index}`, state: 'succeeded' }))
    assert.equal(jobsHaveDependencies(independent), false)

    const dependent = [
      job({ key: 'a', state: 'succeeded' }),
      job({ key: 'b', needs: ['a'], state: 'succeeded' }),
    ]
    assert.equal(jobsHaveDependencies(dependent), true)
  })

  it('merges incremental logs by stable position', () => {
    assert.deepEqual(mergeStepLogs(
      [{ position: 1, text: 'one', byte_length: 3 }, { position: 2, text: 'two', byte_length: 3 }],
      [{ position: 2, text: 'two', byte_length: 3 }, { position: 3, text: 'three', byte_length: 5 }],
    ), {
      logs: [
        { position: 1, text: 'one', byte_length: 3 },
        { position: 2, text: 'two', byte_length: 3 },
        { position: 3, text: 'three', byte_length: 5 },
      ],
      truncated: false,
    })
  })

  it('retains only a bounded suffix of selected step logs', () => {
    const text = 'x'.repeat(300 * 1_024)
    const byte_length = Buffer.byteLength(text)
    assert.deepEqual(mergeStepLogs(
      [{ position: 1, text, byte_length }],
      [{ position: 2, text, byte_length }],
    ), {
      logs: [{ position: 2, text, byte_length }],
      truncated: true,
    })
  })
})

describe('step log pagination', () => {
  const logs = [1, 2, 3].map((position) => ({ position, text: 'log\n', byte_length: 4 }))

  it('does not offer earlier output already retained after an empty or appended refresh', () => {
    const initial = mergeStepLogPage(
      { logs: [], hasEarlier: false },
      { logs: logs.slice(0, 2), has_earlier: false },
      undefined,
    )
    const refreshed = mergeStepLogPage(initial, { logs: [], has_earlier: true }, 2)
    assert.deepEqual(refreshed, initial)
    assert.deepEqual(
      mergeStepLogPage(refreshed, { logs: logs.slice(2), has_earlier: true }, 2),
      { logs, hasEarlier: false },
    )
  })

  it('keeps genuinely earlier output available across refreshes and clears it at the first page', () => {
    const latest = mergeStepLogPage(
      { logs: [], hasEarlier: false },
      { logs: logs.slice(1), has_earlier: true },
      undefined,
    )
    assert.equal(mergeStepLogPage(latest, { logs: [], has_earlier: false }, 3).hasEarlier, true)
    assert.deepEqual(
      mergeStepLogPage(latest, { logs: logs.slice(0, 1), has_earlier: false }, undefined),
      { logs: logs.slice(0, 1), hasEarlier: false },
    )
  })

  it('offers earlier output when the retained window evicts old chunks', () => {
    const text = 'x'.repeat(300 * 1_024)
    const largeLogs = logs.map((log) => ({ ...log, text, byte_length: Buffer.byteLength(text) }))
    assert.deepEqual(
      mergeStepLogPage(
        { logs: largeLogs.slice(0, 1), hasEarlier: false },
        { logs: largeLogs.slice(1, 2), has_earlier: true },
        1,
      ),
      { logs: largeLogs.slice(1, 2), hasEarlier: true },
    )
  })
})

describe('attempt ordering', () => {
  const newestFirst = [
    { id: 'a2', number: 2, steps: [] },
    { id: 'a1', number: 1, steps: [] },
  ]

  it('reads the latest attempt by number, not by position', () => {
    assert.equal(latestAttempt(newestFirst)?.id, 'a2')
    assert.equal(latestAttempt(newestFirst.slice(0, 0))?.id, undefined)
  })

  it('defaults a job to its latest attempt', () => {
    const jobDetail = { attempts: newestFirst, job: { key: 'lint' } }

    assert.equal(attemptForJob(jobDetail, {}, null)?.id, 'a2')
  })
})

describe('run detail navigation', () => {
  const opened = {
    attemptOverrides: {} as Record<string, string>,
    manualSelection: false,
    selectedJobKey: 'build',
    selection: { attemptId: 'a1', jobKey: 'build', stepIndex: 0 },
  }

  it('drops a selection belonging to the job being left', () => {
    const next = selectJob(opened, 'deploy')

    assert.equal(next.selectedJobKey, 'deploy')
    assert.equal(next.selection, null)
    assert.equal(next.manualSelection, true)
  })

  it('keeps the job and its selection when the open job is picked again', () => {
    const next = selectJob(opened, 'build')

    assert.equal(next.selectedJobKey, 'build')
    assert.deepEqual(next.selection, opened.selection)
  })

  it('treats an attempt switch as the reader taking over', () => {
    const next = selectAttempt(opened, 'build', 'a2')

    assert.equal(next.attemptOverrides.build, 'a2')
    assert.equal(next.selection, null)
    assert.equal(next.manualSelection, true)
  })

  it('leaves another job alone when switching attempts', () => {
    const next = selectAttempt(opened, 'deploy', 'a9')

    assert.deepEqual(next.selection, opened.selection)
  })

  it('opens a step in its own job and closes it on a second click', () => {
    const target = { attemptId: 'a3', jobKey: 'deploy', stepIndex: 2 }
    const open = selectStep(opened, target)
    const closed = selectStep(open, target)

    assert.equal(open.selectedJobKey, 'deploy')
    assert.deepEqual(open.selection, target)
    assert.equal(closed.selection, null)
    assert.equal(closed.selectedJobKey, 'deploy')
  })

  it('links a native log line by its attempt and log position, whichever page shows it', () => {
    const log = (position: number, text: string) => ({ position, sequence: position, text, byte_length: text.length, created_at_unix: 1 })
    const firstPage = readStepLogLines([log(4, 'compiling\n'), log(5, 'test a ok\nAssertionError: '), log(6, 'expected 3\nfailed\n')])
    const latestPage = readStepLogLines([log(5, 'test a ok\nAssertionError: '), log(6, 'expected 3\nfailed\n')])
    const midLinePage = readStepLogLines([log(6, 'expected 3\nfailed\n')])
    const failure = firstPage.find((line) => line.text === 'AssertionError: expected 3')!
    assert.deepEqual(firstPage.map((line) => line.anchor), ['4.0', '5.0', '5.1', '6.1'])
    for (const page of [latestPage, midLinePage]) {
      for (const line of page) {
        const named = firstPage.find((first) => first.anchor === line.anchor)
        if (named) assert.equal(line.text, named.text, line.anchor)
      }
    }
    assert.equal(latestPage.find((line) => line.anchor === failure.anchor)?.text, failure.text)
    assert.equal(midLinePage.find((line) => line.anchor === '6.1')?.text, 'failed')

    const attempt = (id: string, number: number): RepositoryRunAttemptResponse => ({
      id, number, external_run_id: null, runtime_version: '1', state: 'failed', created_at_unix: 1, started_at_unix: 1,
      completed_at_unix: 2, terminal_reason: null, cache_setup: null, caches: [],
      steps: [{ index: 0, name: 'Test', command: 'make test', state: 'failed', started_at_unix: 1, completed_at_unix: 2, exit_code: 1 }],
    })
    const detail: Pick<RepositoryRunDetailResponse, 'jobs' | 'opening'> = {
      jobs: [{
        job: { key: 'test', needs: [], pinned_container_image: 'image', state: 'failed', created_at_unix: 1, started_at_unix: 1, updated_at_unix: 2, completed_at_unix: 2 },
        attempts: [attempt('first', 1), attempt('retry', 2)],
        opening_step: { attempt_id: 'retry', step_index: 0 },
      }],
      opening: null,
    }
    const open = (attemptId: string) => openingView(detail, runJobLinkForHash(['test'], `#${runLogLineId('test', attemptId, 0, failure.anchor)}`))
    assert.deepEqual(open('first').selection, { attemptId: 'first', jobKey: 'test', stepIndex: 0 })
    assert.deepEqual(open('retired').selection, { attemptId: 'retry', jobKey: 'test', stepIndex: 0 })
  })
})
