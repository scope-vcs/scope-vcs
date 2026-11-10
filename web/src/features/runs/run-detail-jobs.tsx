import { useMemo, useRef } from 'react'
import type {
  StepLogs,
  StepSelection,
} from './repository-run-detail-controller'
import { attemptForJob, jobsHaveDependencies } from './repository-run-detail-model'
import { RunDetailSteps } from './run-detail-steps'
import { runJobPanelId, type RunJobLink } from './run-job-ids'
import { RunJobGraph } from './run-job-graph'
import { orderJobsByDependency } from './run-job-graph-model'
import { RunJobTree } from './run-job-tree'
import { startRunJobTiming, useRunLogAbsence } from './run-log-timing'
import { RUN_BODY_CLASS } from './run-layout'
import type { RepositoryRunJobDetailResponse } from '@/api/types.generated'

export function RunDetailJobs({
  attemptOverrides,
  jobs,
  link,
  onSelectAttempt,
  onSelectJob,
  onSelectStep,
  onToggleGraph,
  runId,
  selectedJobKey,
  selection,
  showGraph,
  stepLogs,
}: {
  attemptOverrides: Readonly<Record<string, string>>
  jobs: readonly RepositoryRunJobDetailResponse[]
  link: RunJobLink | null
  onSelectAttempt: (jobKey: string, attemptId: string) => void
  onSelectJob: (job: RepositoryRunJobDetailResponse) => void
  onSelectStep: (jobKey: string, attemptId: string, stepIndex: number) => void
  onToggleGraph: () => void
  runId: string
  selectedJobKey: string | null
  selection: StepSelection | null
  showGraph: boolean
  stepLogs: StepLogs
}) {
  const selectedJob = jobs.find(({ job }) => job.key === selectedJobKey) ?? null
  const orderedJobs = useMemo(() => orderJobsByDependency(jobs), [jobs])
  const summaries = useMemo(() => orderedJobs.map(({ job }) => ({
    key: job.key,
    name: job.key,
    state: job.state,
    startedAt: job.started_at_unix,
    completedAt: job.completed_at_unix,
  })), [orderedJobs])
  const graphShown = jobsHaveDependencies(jobs) && showGraph
  useRunLogAbsence(runId, graphShown)
  const treeRef = useRef<HTMLDivElement>(null)
  function pickJob(key: string) {
    const job = jobs.find((listed) => listed.job.key === key)
    if (!job) return
    if (key !== selectedJobKey) startRunJobTiming(runId)
    onSelectJob(job)
    if (showGraph) onToggleGraph()
  }
  function pickJobFromGraph(job: RepositoryRunJobDetailResponse) {
    pickJob(job.job.key)
    requestAnimationFrame(() => {
      treeRef.current
        ?.querySelector<HTMLButtonElement>('button[aria-pressed="true"]')
        ?.focus()
    })
  }

  return (
    <div className={RUN_BODY_CLASS}>
      <div className="contents" ref={treeRef}>
        <RunJobTree
          footer={jobsHaveDependencies(jobs) ? (
            <button
              aria-pressed={showGraph}
              className="h-9 shrink-0 px-3 text-xs text-muted-foreground underline-offset-2 hover:text-foreground hover:underline lg:h-auto lg:px-4 lg:py-2 lg:text-left"
              onClick={onToggleGraph}
              type="button"
            >
              {showGraph ? 'Hide graph' : 'Show graph'}
            </button>
          ) : null}
          jobs={summaries}
          onSelect={pickJob}
          selectedKey={selectedJobKey}
        />
      </div>
      <div className="flex min-w-0 flex-col lg:min-h-0">
        {graphShown ? (
          <RunJobGraph
            jobs={jobs}
            onSelectJob={pickJobFromGraph}
            selectedJobKey={selectedJobKey}
          />
        ) : null}
        {selectedJob ? (
          <div
            className={graphShown ? 'hidden' : 'flex flex-col lg:min-h-0 lg:flex-1'}
            id={runJobPanelId(selectedJob.job.key)}
          >
            <RunDetailSteps
              key={selectedJob.job.key}
              attempt={attemptForJob(selectedJob, attemptOverrides, selection)}
              jobDetail={selectedJob}
              link={link}
              onSelectAttempt={(attemptId) =>
                onSelectAttempt(selectedJob.job.key, attemptId)}
              onSelectStep={(attemptId, stepIndex) =>
                onSelectStep(selectedJob.job.key, attemptId, stepIndex)}
              runId={runId}
              selection={selection}
              stepLogs={stepLogs}
            />
          </div>
        ) : graphShown ? null : (
          <p className="px-4 py-6 text-sm text-muted-foreground">
            {jobs.length === 0 ? 'This workflow has no jobs.' : 'Select a job to see its steps.'}
          </p>
        )}
      </div>
    </div>
  )
}
