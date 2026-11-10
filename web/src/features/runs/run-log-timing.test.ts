import assert from 'node:assert/strict'
import test from 'node:test'
import type { RunLogTiming } from '@/analytics/diagnostics'
import { createRunLogTimer } from './run-log-timing'

function timer(documentRoute = false) {
  const reports: RunLogTiming[] = []
  let frames: Array<(paintedAt: number) => void> = []
  const runLog = createRunLogTimer({
    afterPaint: (paint) => {
      frames.push(paint)
      return () => { frames = frames.filter((frame) => frame !== paint) }
    },
    isDocumentRoute: () => documentRoute,
    report: (timing) => reports.push(timing),
  })
  const paint = (paintedAt: number) => {
    const due = frames
    frames = []
    for (const frame of due) frame(paintedAt)
  }
  return { paint, reports, runLog }
}

test('a run log intent reports once, after paint, with the UTF-8 size of the painted text', () => {
  const { paint, reports, runLog } = timer()
  runLog.start('list_click', 'run-1', 1000.2)
  runLog.enterRunPage('run-1')
  assert.equal(runLog.commitText('run-1', 'native', ''), null)
  runLog.commitText('run-1', 'native', 'ok ✔\n')
  assert.deepEqual(reports, [], 'nothing is reported before the text paints')
  paint(1412.4)
  paint(1500)
  assert.equal(runLog.commitText('run-1', 'native', 'more'), null)

  assert.deepEqual(reports, [{ durationMs: 412, logBytes: 7, provider: 'native', source: 'stored', trigger: 'list_click' }])
})

test('a run log intent is dropped when its view closes before paint, the user leaves, opens another run, picks another job first, or the log settles without text', () => {
  const { paint, reports, runLog } = timer()
  runLog.start('job_click', 'run-1', 0)
  runLog.settleWithoutText('run-1')
  assert.equal(runLog.commitText('run-1', 'github', 'log after the job finished'), null)

  runLog.start('job_click', 'run-1', 5)
  const closeView = runLog.commitText('run-1', 'github', 'log')
  closeView?.()
  paint(8)
  assert.equal(runLog.commitText('run-1', 'github', 'log in a later view'), null)

  runLog.start('job_click', 'run-1', 10)
  runLog.commitText('run-1', 'github', 'log')
  runLog.leaveRunPage('run-1')
  paint(50)

  runLog.start('list_click', 'run-1', 60)
  runLog.enterRunPage('run-2')
  assert.equal(runLog.commitText('run-2', 'github', 'log'), null)

  runLog.start('job_click', 'run-2', 100)
  const closeFirstJob = runLog.commitText('run-2', 'github', 'first job')
  runLog.start('job_click', 'run-2', 120)
  closeFirstJob?.()
  paint(130)
  assert.deepEqual(reports, [])
  runLog.commitText('run-2', 'github', 'second job')
  paint(200)

  assert.deepEqual(reports.map(({ durationMs, trigger }) => ({ durationMs, trigger })), [{ durationMs: 80, trigger: 'job_click' }])
})

test('the document run route measures from navigation start once and counts pending answers as fetched', () => {
  const { paint, reports, runLog } = timer(true)
  runLog.observeFetching('run-1')
  runLog.commitText('run-1', 'github', 'log')
  paint(640)
  runLog.leaveRunPage('run-1')
  runLog.enterRunPage('run-1')
  assert.equal(runLog.commitText('run-1', 'github', 'log'), null)
  runLog.start('job_click', 'run-1', 900)
  runLog.commitText('run-1', 'github', 'log')
  paint(950)

  assert.deepEqual(reports.map(({ durationMs, source, trigger }) => ({ durationMs, source, trigger })), [
    { durationMs: 640, source: 'fetched', trigger: 'page_load' },
    { durationMs: 50, source: 'stored', trigger: 'job_click' },
  ])
})
