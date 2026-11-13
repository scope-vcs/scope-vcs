import assert from 'node:assert/strict'
import test from 'node:test'
import type { RunLogTiming } from '@/analytics/diagnostics'
import { afterNextPaint, createRunLogTimer } from './run-log-timing'

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
  assert.equal(runLog.commitText('run-1', ''), null)
  runLog.commitText('run-1', 'ok ✔\n')
  assert.deepEqual(reports, [], 'nothing is reported before the text paints')
  paint(1412.4)
  paint(1500)
  assert.equal(runLog.commitText('run-1', 'more'), null)

  assert.deepEqual(reports, [{ durationMs: 412, logBytes: 7, source: 'stored', trigger: 'list_click' }])
})

test('a run log intent is dropped when its view closes before paint, the user leaves, opens another run, picks another job first, or the log settles without text', () => {
  const { paint, reports, runLog } = timer()
  runLog.start('job_click', 'run-1', 0)
  runLog.settleWithoutText('run-1')
  assert.equal(runLog.commitText('run-1', 'log after the job finished'), null)

  runLog.start('job_click', 'run-1', 5)
  const closeView = runLog.commitText('run-1', 'log')
  closeView?.()
  paint(8)
  assert.equal(runLog.commitText('run-1', 'log in a later view'), null)

  runLog.start('job_click', 'run-1', 10)
  runLog.commitText('run-1', 'log')
  runLog.leaveRunPage('run-1')
  paint(50)

  runLog.start('list_click', 'run-1', 60)
  runLog.enterRunPage('run-2')
  assert.equal(runLog.commitText('run-2', 'log'), null)

  runLog.start('job_click', 'run-2', 100)
  const closeFirstJob = runLog.commitText('run-2', 'first job')
  runLog.start('job_click', 'run-2', 120)
  closeFirstJob?.()
  paint(130)
  assert.deepEqual(reports, [])
  runLog.commitText('run-2', 'second job')
  paint(200)

  assert.deepEqual(reports.map(({ durationMs, trigger }) => ({ durationMs, trigger })), [{ durationMs: 80, trigger: 'job_click' }])
})

test('the document run route measures from navigation start once and counts pending answers as fetched', () => {
  const { paint, reports, runLog } = timer(true)
  runLog.observeFetching('run-1')
  runLog.commitText('run-1', 'log')
  paint(640)
  runLog.leaveRunPage('run-1')
  runLog.enterRunPage('run-1')
  assert.equal(runLog.commitText('run-1', 'log'), null)
  runLog.start('job_click', 'run-1', 900)
  runLog.commitText('run-1', 'log')
  paint(950)

  assert.deepEqual(reports.map(({ durationMs, source, trigger }) => ({ durationMs, source, trigger })), [
    { durationMs: 640, source: 'fetched', trigger: 'page_load' },
    { durationMs: 50, source: 'stored', trigger: 'job_click' },
  ])
})

test('a log view that closes before its text drops the open it was showing, but not a newer click', () => {
  const { paint, reports, runLog } = timer()
  const firstView = {}
  runLog.start('list_click', 'run-1', 0)
  runLog.claim('run-1', firstView)
  runLog.release(firstView)
  assert.equal(runLog.commitText('run-1', 'log of a job opened from the URL hash'), null)

  const secondView = {}
  runLog.start('job_click', 'run-1', 10)
  runLog.claim('run-1', secondView)
  runLog.start('job_click', 'run-1', 20)
  runLog.release(secondView)
  runLog.claim('run-1', {})
  runLog.commitText('run-1', 'log of the clicked job')
  paint(70)

  assert.deepEqual(reports.map(({ durationMs, trigger }) => ({ durationMs, trigger })), [{ durationMs: 50, trigger: 'job_click' }])
})

test('the page reports after the next frame has painted, not when the text is committed', async () => {
  const frames: Array<() => void> = []
  const browser = globalThis as { requestAnimationFrame?: unknown, cancelAnimationFrame?: unknown }
  browser.requestAnimationFrame = (frame: () => void) => frames.push(frame)
  browser.cancelAnimationFrame = () => {}
  try {
    let paintedAt: number | null = null
    afterNextPaint((at) => { paintedAt = at })
    assert.equal(paintedAt, null, 'nothing runs before the frame')
    frames.splice(0).forEach((frame) => frame())
    assert.equal(paintedAt, null, 'the frame callback runs before paint, so the report waits one more task')
    await new Promise((resolve) => setTimeout(resolve))
    assert.equal(typeof paintedAt, 'number')
  } finally {
    delete browser.requestAnimationFrame
    delete browser.cancelAnimationFrame
  }
})
