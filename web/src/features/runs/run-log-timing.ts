import { useLayoutEffect, useRef, type MouseEvent } from 'react'
import { captureRunLogTiming, type RunLogTiming } from '../../analytics/diagnostics'

type RunLogIntent = {
  runId: string
  trigger: RunLogTiming['trigger']
  startedAt: number
  fetched: boolean
}

export function createRunLogTimer({
  isDocumentRoute,
  report,
}: {
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
    commitText(runId: string, provider: RunLogTiming['provider'], text: string) {
      const committed = text ? intentFor(runId) : null
      if (!committed) return null
      return (paintedAt: number) => {
        if (intent !== committed) return
        intent = null
        report({
          durationMs: Math.round(Math.max(0, paintedAt - committed.startedAt)),
          logBytes: new TextEncoder().encode(text).length,
          provider,
          source: committed.fetched ? 'fetched' : 'stored',
          trigger: committed.trigger,
        })
      }
    },
  }
}

const runLogTimer = createRunLogTimer({
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

export function useRunLogAbsence(runId: string, absent: boolean) {
  useLayoutEffect(() => {
    if (absent) runLogTimer.settleWithoutText(runId)
  }, [absent, runId])
}

export function useRunLogPaintTiming({
  fetching = false,
  provider,
  runId,
  settled,
  text,
}: {
  fetching?: boolean
  provider: RunLogTiming['provider']
  runId: string
  settled: boolean
  text: string
}) {
  const painted = useRef(false)
  useLayoutEffect(() => {
    if (fetching) runLogTimer.observeFetching(runId)
    if (painted.current) return
    if (!text) {
      if (settled) runLogTimer.settleWithoutText(runId)
      return
    }
    painted.current = true
    const finish = runLogTimer.commitText(runId, provider, text)
    if (!finish) return
    requestAnimationFrame(() => setTimeout(() => finish(performance.now())))
  }, [fetching, provider, runId, settled, text])
}
