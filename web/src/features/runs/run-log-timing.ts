import { useLayoutEffect, useRef, useState, type MouseEvent } from 'react'
import { captureRunLogTiming, type RunLogTiming } from '../../analytics/diagnostics'

type RunLogIntent = {
  runId: string
  trigger: RunLogTiming['trigger']
  startedAt: number
  fetched: boolean
  view?: object
}

type AfterPaint = (paint: (paintedAt: number) => void) => () => void

export function createRunLogTimer({
  afterPaint,
  isDocumentRoute,
  report,
}: {
  afterPaint: AfterPaint
  isDocumentRoute: () => boolean
  report: (timing: RunLogTiming) => void
}) {
  let intent: RunLogIntent | null = null
  let documentRouteClaimed = false

  function intentFor(runId: string) {
    if (intent && intent.runId !== runId) intent = null
    if (!intent && !documentRouteClaimed && isDocumentRoute()) {
      intent = { runId, trigger: 'page_load', startedAt: 0, fetched: false }
    }
    documentRouteClaimed = true
    return intent
  }

  return {
    start(trigger: 'job_click' | 'list_click', runId: string, startedAt: number) {
      documentRouteClaimed = true
      intent = { runId, trigger, startedAt, fetched: false }
    },
    enterRunPage(runId: string) {
      intentFor(runId)
    },
    claim(runId: string, view: object) {
      const current = intentFor(runId)
      if (current && !current.view) current.view = view
    },
    release(view: object) {
      if (intent?.view === view) intent = null
    },
    leaveRunPage(runId: string) {
      if (intent?.runId === runId) intent = null
    },
    settleWithoutText(runId: string) {
      if (intentFor(runId)) intent = null
    },
    observeFetching(runId: string) {
      const current = intentFor(runId)
      if (current) current.fetched = true
    },
    commitText(runId: string, text: string) {
      const committed = text ? intentFor(runId) : null
      if (!committed) return null
      const cancelPaint = afterPaint((paintedAt) => {
        if (intent !== committed) return
        intent = null
        report({
          durationMs: Math.round(Math.max(0, paintedAt - committed.startedAt)),
          logBytes: new TextEncoder().encode(text).length,
          source: committed.fetched ? 'fetched' : 'stored',
          trigger: committed.trigger,
        })
      })
      return () => {
        cancelPaint()
        if (intent === committed) intent = null
      }
    },
  }
}

export function afterNextPaint(paint: (paintedAt: number) => void) {
  let task: ReturnType<typeof setTimeout> | undefined
  const frame = requestAnimationFrame(() => {
    task = setTimeout(() => paint(performance.now()))
  })
  return () => {
    cancelAnimationFrame(frame)
    clearTimeout(task)
  }
}

const runLogTimer = createRunLogTimer({
  afterPaint: afterNextPaint,
  isDocumentRoute: () => {
    const [navigation] = performance.getEntriesByType('navigation')
    return navigation !== undefined && new URL(navigation.name).pathname === window.location.pathname
  },
  report: captureRunLogTiming,
})

export function startRunListTiming(event: MouseEvent, runId: string) {
  if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
  runLogTimer.start('list_click', runId, performance.now())
}

export function startRunJobTiming(runId: string) {
  runLogTimer.start('job_click', runId, performance.now())
}

export function useRunLogTimingPage(runId: string) {
  useLayoutEffect(() => {
    runLogTimer.enterRunPage(runId)
    return () => runLogTimer.leaveRunPage(runId)
  }, [runId])
}

export function observeRunLogFetching(runId: string) {
  runLogTimer.observeFetching(runId)
}

export function useRunLogAbsence(runId: string, absent: boolean) {
  useLayoutEffect(() => {
    if (absent) runLogTimer.settleWithoutText(runId)
  }, [absent, runId])
}

export function useRunLogPaintTiming({
  runId,
  settled,
  text,
}: {
  runId: string
  settled: boolean
  text: string
}) {
  const closeView = useRef<(() => void) | null>(null)
  const [view] = useState(() => ({}))
  useLayoutEffect(() => {
    runLogTimer.claim(runId, view)
    return () => {
      closeView.current?.()
      runLogTimer.release(view)
    }
  }, [runId, view])
  useLayoutEffect(() => {
    if (closeView.current) return
    if (!text) {
      if (settled) runLogTimer.settleWithoutText(runId)
      return
    }
    closeView.current = runLogTimer.commitText(runId, text) ?? (() => {})
  }, [runId, settled, text])
}
