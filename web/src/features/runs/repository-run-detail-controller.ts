import type { RunActionInput, RunStepLogsInput } from '@/api/types'
import type {
  RepositoryRunDetailResponse,
  RepositoryRunJobDetailResponse,
  RepositoryRunStepLogPageResponse,
} from '@/api/types.generated'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import {
  useCallback,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  defaultShowGraph,
  reconcileAttemptOverrides,
  selectAttempt as selectAttemptInJob,
  selectJob,
  selectStep,
  runCanChange,
  selectInitialView,
  type StepSelection,
} from './repository-run-detail-model'
import { useRunLiveRefresh, type RunRefresh } from './run-live-refresh'
import {
  canReuseRunLogs,
  completedRunLogVersion,
  EMPTY_LOG_STATE,
  stepKey,
  runLogsResource,
  refreshRunLogs,
  refreshRunLogsAfterInFlight,
  type RunLogMode,
  type StepLogState,
} from './run-log-cache'

import { initializeRunDetail, refreshRunDetail, runDetailResource } from './run-detail-resource'

export type { StepSelection } from './repository-run-detail-model'
export type { StepLogState } from './run-log-cache'

const DETAIL_CHANGES = ['StatusChanged', 'LogsAppended'] as const
const RUN_ERROR_FALLBACK = 'Run operation failed.'

/** The selected step's output plus the actions the log view can take on it. */
export type StepLogs = {
  earlier: () => void
  latest: () => void
  retry: () => void
  state: StepLogState
}

type DetailViewState = {
  actionError: string | null
  attemptOverrides: Record<string, string>
  manualSelection: boolean
  pendingAction: 'cancel' | 'retry' | null
  reconciliationGeneration: number | null
  selectedJobKey: string | null
  selection: StepSelection | null
  showGraph: boolean
}

function createDetailViewState(detail: RepositoryRunDetailResponse): DetailViewState {
  const initialView = selectInitialView(detail.jobs)
  return {
    actionError: null,
    attemptOverrides: {},
    manualSelection: false,
    pendingAction: null,
    reconciliationGeneration: null,
    selectedJobKey: initialView.selectedJobKey,
    selection: initialView.selection,
    showGraph: defaultShowGraph(detail.jobs),
  }
}

export function useRepositoryRunDetailController({
  cacheKey,
  initialDetail,
  loadDetail,
  loadLogs,
  params,
}: {
  cacheKey: string | null
  initialDetail: RepositoryRunDetailResponse
  loadDetail: (signal?: AbortSignal) => Promise<RepositoryRunDetailResponse>
  loadLogs: (
    input: RunStepLogsInput,
    signal?: AbortSignal,
  ) => Promise<RepositoryRunStepLogPageResponse>
  params: RunActionInput
}) {
  const [key] = useState(() => cacheKey ?? crypto.randomUUID())
  useState(() => {
    initializeRunDetail(key, initialDetail)
    runLogsResource.read(key)
  })
  const detailSnapshot = useSyncExternalStore(
    useCallback((listener) => runDetailResource.subscribe(key, listener), [key]),
    useCallback(() => runDetailResource.getSnapshot(key), [key]),
    runDetailResource.getServerSnapshot,
  )
  const logsSnapshot = useSyncExternalStore(
    useCallback((listener) => runLogsResource.subscribe(key, listener), [key]),
    useCallback(() => runLogsResource.getSnapshot(key), [key]),
    runLogsResource.getServerSnapshot,
  )
  const detail = detailSnapshot.value?.detail ?? initialDetail
  const [view, updateView] = useState(() => createDetailViewState(detail))
  const selectionRef = useRef(view.selection)
  useEffect(() => { selectionRef.current = view.selection }, [view.selection])

  useEffect(() => {
    updateView((current) => {
      const reconciledAction = current.reconciliationGeneration !== null &&
        (detailSnapshot.value?.generation ?? 0) >= current.reconciliationGeneration
      const selectionStillValid = current.selection !== null && selectionExists(current.selection, detail.jobs)
      const initialView = current.manualSelection ? null : selectInitialView(detail.jobs)
      const selection = selectionStillValid ? current.selection
        : current.manualSelection ? null : initialView?.selection ?? null
      return {
        ...current,
        attemptOverrides: reconcileAttemptOverrides(current.attemptOverrides, detail.jobs),
        pendingAction: reconciledAction ? null : current.pendingAction,
        reconciliationGeneration: reconciledAction ? null : current.reconciliationGeneration,
        selectedJobKey: selection ? selection.jobKey
          : current.manualSelection
            ? jobExists(current.selectedJobKey, detail.jobs) ? current.selectedJobKey : null
            : initialView?.selectedJobKey ?? null,
        selection,
      }
    })
  }, [detail, detailSnapshot.value?.generation])

  const refreshDetail = useCallback((forceAfterInFlight = false) =>
    refreshRunDetail(key, loadDetail, forceAfterInFlight), [key, loadDetail])
  const refreshLogs = useCallback((target: StepSelection, mode: RunLogMode = 'refresh') =>
    refreshRunLogs({ key, target, params, loadLogs, mode, detail: runDetailResource.peek(key)?.detail ?? initialDetail }),
  [key, params, loadLogs, initialDetail])
  const refreshLogsAfterInFlight = useCallback((target: StepSelection) =>
    refreshRunLogsAfterInFlight({ key, target, params, loadLogs, getDetail: () => runDetailResource.peek(key)?.detail ?? initialDetail }),
  [key, params, loadLogs, initialDetail])

  const refreshFromRunEvents = useCallback<RunRefresh>(async (
    reasons,
  ) => {
    const refreshMetadata = reasons.has('Recovery') ||
      reasons.has('StatusChanged')
    if (refreshMetadata) await refreshDetail()
    const selection = selectionRef.current
    if (selection && (refreshMetadata || reasons.has('LogsAppended'))) {
      if (!await refreshLogsAfterInFlight(selection)) {
        throw new Error('Selected run logs could not refresh.')
      }
    }
  }, [refreshDetail, refreshLogsAfterInFlight])

  const refreshRun = useRunLiveRefresh({
    acceptedChanges: DETAIL_CHANGES,
    mutable: runCanChange(detail.run.state),
    refresh: refreshFromRunEvents,
    runId: params.run_id,
  })

  const completedVersion = completedRunLogVersion(detail)
  useEffect(() => {
    const selection = view.selection
    if (!selection) return
    const cached = runLogsResource.peek(key)?.[stepKey(selection)]
    if (cached && canReuseRunLogs(cached, detail)) return
    if (!runCanChange(detail.run.state)) {
      void refreshLogsAfterInFlight(selection)
      return
    }
    void refreshLogs(selection)
  }, [
    refreshLogs,
    refreshLogsAfterInFlight,
    completedVersion,
    detail,
    key,
    view.selection,
  ])

  const performAction = useCallback(async (
    kind: 'cancel' | 'retry',
    action: () => Promise<void>,
  ) => {
    updateView((current) => ({
      ...current,
      actionError: null,
      pendingAction: kind,
    }))
    try {
      await action()
    } catch (error) {
      updateView((current) => ({
        ...current,
        actionError: resourceErrorMessage(error, RUN_ERROR_FALLBACK),
        pendingAction: null,
      }))
      return
    }
    const snapshot = runDetailResource.getSnapshot(key)
    const reconciliationGeneration = Number(snapshot.version ?? 0) + 1
    updateView((current) => ({
      ...current,
      reconciliationGeneration,
    }))
    try {
      await refreshDetail(true)
    } catch {
      // The detail loader owns metadata errors. Keep controls disabled until a
      // post-mutation refresh reaches the required generation.
    }
  }, [key, refreshDetail])

  // Navigation rules live in the model so `selection` and `selectedJobKey`
  // cannot drift apart here.
  function toggleJob(jobDetail: RepositoryRunJobDetailResponse) {
    updateView((current) => selectJob(current, jobDetail.job.key))
  }

  function selectAttempt(jobKey: string, attemptId: string) {
    updateView((current) => selectAttemptInJob(current, jobKey, attemptId))
  }

  function toggleStep(jobKey: string, attemptId: string, stepIndex: number) {
    updateView((current) => selectStep(current, { attemptId, jobKey, stepIndex }))
  }

  function toggleGraph() {
    updateView((current) => ({ ...current, showGraph: !current.showGraph }))
  }

  const selection = view.selection
  const stepLogs: StepLogs = {
    earlier: () => { if (selection) void refreshLogs(selection, 'earlier') },
    latest: () => { if (selection) void refreshLogs(selection, 'latest') },
    retry: () => { if (selection) void refreshLogs(selection, 'retry') },
    state: selection
      ? logsSnapshot.value?.[stepKey(selection)] ?? EMPTY_LOG_STATE
      : EMPTY_LOG_STATE,
  }

  return {
    ...view,
    detail,
    metadataError: detailSnapshot.error === null
      ? null
      : resourceErrorMessage(detailSnapshot.error, RUN_ERROR_FALLBACK),
    performAction,
    refreshDetail: refreshRun,
    selectAttempt,
    stepLogs,
    toggleGraph,
    toggleJob,
    toggleStep,
  }
}

function selectionExists(
  selection: StepSelection,
  jobs: readonly RepositoryRunJobDetailResponse[],
) {
  return jobs.some(({ job, attempts }) =>
    job.key === selection.jobKey &&
    attempts.some((attempt) =>
      attempt.id === selection.attemptId &&
      attempt.steps.some((step) => step.index === selection.stepIndex)
    )
  )
}

function jobExists(jobKey: string | null, jobs: readonly RepositoryRunJobDetailResponse[]) {
  return jobKey !== null && jobs.some(({ job }) => job.key === jobKey)
}
