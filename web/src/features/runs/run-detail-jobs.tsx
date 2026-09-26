import { useMemo } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import type {
  StepLogs,
  StepSelection,
} from './repository-run-detail-controller'
import { attemptForJob } from './repository-run-detail-model'
import { RunDetailSteps } from './run-detail-steps'
import { RunDuration } from './run-duration'
import { RUN_JOB_LIST_CLASS, RUN_JOB_ROW_CLASS } from './run-job-layout'
import { runJobPanelId } from './run-job-ids'
import { RunJobGraph } from './run-job-graph'
import { orderJobsByDependency } from './run-job-graph-model'
import { RunStatusIcon } from './run-status-icon'
import type { RepositoryRunJobDetailResponse } from '@/api/types.generated'

/** The run's working area: the job list beside the selected job's steps. At
 * desktop widths the steps pane is the page's only scroller, so a long log
 * never nests one scrollbar inside another. */
export function RunDetailJobs({
  attemptOverrides,
  jobs,
  onSelectAttempt,
  onSelectJob,
  onSelectStep,
  onToggleGraph,
  selectedJobKey,
  selection,
  showGraph,
  stepLogs,
}: {
  attemptOverrides: Readonly<Record<string, string>>
  jobs: readonly RepositoryRunJobDetailResponse[]
  onSelectAttempt: (jobKey: string, attemptId: string) => void
  onSelectJob: (job: RepositoryRunJobDetailResponse) => void
  onSelectStep: (jobKey: string, attemptId: string, stepIndex: number) => void
  onToggleGraph: () => void
  selectedJobKey: string | null
  selection: StepSelection | null
  showGraph: boolean
  stepLogs: StepLogs
}) {
  const selectedJob = jobs.find(({ job }) => job.key === selectedJobKey) ?? null
  const orderedJobs = useMemo(() => orderJobsByDependency(jobs), [jobs])

  return (
    <div className="flex min-h-0 flex-1 flex-col border-t border-border lg:grid lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-[minmax(0,1fr)]">
      <nav
        aria-labelledby="jobs-heading"
        className="min-w-0 border-b border-border lg:overflow-y-auto lg:border-b-0 lg:border-r"
      >
        <div className="flex items-center justify-between gap-2 px-4 pt-3">
          <h2 className="text-sm font-semibold" id="jobs-heading">
            Jobs
          </h2>
          <Button
            aria-pressed={showGraph}
            onClick={onToggleGraph}
            size="sm"
            variant={showGraph ? 'secondary' : 'ghost'}
          >
            Graph
          </Button>
        </div>
        <p className="px-4 text-xs text-muted-foreground">{jobSummary(jobs)}</p>
        <RunJobList
          jobs={orderedJobs}
          onSelectJob={onSelectJob}
          selectedJobKey={selectedJobKey}
        />
      </nav>
      {/* Keyed by job so a newly picked job starts at its top instead of
          inheriting the previous job's scroll position. */}
      <div className="min-w-0 lg:overflow-y-auto" key={selectedJobKey ?? 'none'}>
        {showGraph ? (
          <RunJobGraph
            jobs={jobs}
            onSelectJob={onSelectJob}
            selectedJobKey={selectedJobKey}
          />
        ) : null}
        {selectedJob ? (
          <div id={runJobPanelId(selectedJob.job.key)}>
            <RunDetailSteps
              attempt={attemptForJob(selectedJob, attemptOverrides, selection)}
              jobDetail={selectedJob}
              onSelectAttempt={(attemptId) =>
                onSelectAttempt(selectedJob.job.key, attemptId)}
              onSelectStep={(attemptId, stepIndex) =>
                onSelectStep(selectedJob.job.key, attemptId, stepIndex)}
              selection={selection}
              stepLogs={stepLogs}
            />
          </div>
        ) : (
          <p className="px-4 py-6 text-sm text-muted-foreground">
            {jobs.length === 0 ? 'This workflow has no jobs.' : 'Select a job to see its steps.'}
          </p>
        )}
      </div>
    </div>
  )
}

function RunJobList({
  jobs,
  onSelectJob,
  selectedJobKey,
}: {
  jobs: readonly RepositoryRunJobDetailResponse[]
  onSelectJob: (job: RepositoryRunJobDetailResponse) => void
  selectedJobKey: string | null
}) {
  return (
    <ul className={RUN_JOB_LIST_CLASS}>
      {jobs.map((jobDetail) => {
        const { job } = jobDetail
        const selected = job.key === selectedJobKey
        return (
          <li key={job.key}>
            <button
              aria-controls={runJobPanelId(job.key)}
              aria-pressed={selected}
              className={cn(
                RUN_JOB_ROW_CLASS,
                'outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
                selected && 'bg-muted font-medium lg:before:absolute lg:before:inset-y-1.5 lg:before:left-0 lg:before:w-0.5 lg:before:rounded-full lg:before:bg-foreground',
              )}
              onClick={() => onSelectJob(jobDetail)}
              type="button"
            >
              <RunStatusIcon state={job.state} />
              <span className="min-w-0 flex-1 truncate">{job.key}</span>
              <span className="text-xs font-normal text-muted-foreground">
                <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />
              </span>
            </button>
          </li>
        )
      })}
    </ul>
  )
}

function jobSummary(jobs: readonly RepositoryRunJobDetailResponse[]) {
  const counts = new Map<string, number>()
  for (const { job } of jobs) {
    counts.set(job.state, (counts.get(job.state) ?? 0) + 1)
  }
  return [...counts.entries()]
    .map(([state, count]) => `${count} ${state}`)
    .join(' · ') || 'No jobs'
}
