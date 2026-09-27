import { cn } from '@/lib/utils'
import { useMemo } from 'react'
import {
  JOB_GRAPH_NODE_HEIGHT,
  JOB_GRAPH_NODE_WIDTH,
  buildRunJobGraph,
} from './run-job-graph-model'
import { RunDuration } from './run-duration'
import { RunStatusIcon } from './run-status-icon'
import type { RepositoryRunJobDetailResponse } from '@/api/types.generated'

/** Jobs laid out by what they wait on. Each node carries only what the job
 * list does: status, name and duration; the arrows say the rest. */
export function RunJobGraph({
  jobs,
  onSelectJob,
  selectedJobKey,
}: {
  jobs: readonly RepositoryRunJobDetailResponse[]
  onSelectJob: (job: RepositoryRunJobDetailResponse) => void
  selectedJobKey: string | null
}) {
  const layout = useMemo(() => buildRunJobGraph(jobs), [jobs])
  const jobsByKey = new Map(jobs.map((job) => [job.job.key, job]))

  return (
    <div
      aria-label="Job dependency graph"
      className="overflow-auto bg-muted/15 lg:min-h-0 lg:flex-1"
    >
      <div
        className="relative"
        style={{ height: layout.height, minWidth: layout.width, width: layout.width }}
      >
        <svg
          aria-hidden="true"
          className="absolute inset-0 size-full overflow-visible"
          viewBox={`0 0 ${layout.width} ${layout.height}`}
        >
          <defs>
            <marker
              id="run-job-edge-arrow"
              markerHeight="6"
              markerWidth="6"
              orient="auto-start-reverse"
              refX="5"
              refY="3"
            >
              <path className="fill-muted-foreground/70" d="M 0 0 L 6 3 L 0 6 z" />
            </marker>
          </defs>
          {layout.edges.map((edge) => (
            <path
              className="fill-none stroke-border"
              d={edge.path}
              key={edge.key}
              markerEnd="url(#run-job-edge-arrow)"
              strokeWidth="2"
            />
          ))}
        </svg>
        {layout.nodes.map((node) => {
          const jobDetail = jobsByKey.get(node.key)
          if (!jobDetail) return null
          const { job } = jobDetail
          return (
            <button
              aria-pressed={selectedJobKey === job.key}
              className={cn(
                'absolute flex items-center gap-2 border bg-background px-3 text-left text-sm shadow-sm outline-none transition-colors hover:border-foreground/35 hover:bg-muted/20 focus-visible:ring-2 focus-visible:ring-ring',
                selectedJobKey === job.key && 'border-foreground/50 ring-1 ring-foreground/10',
              )}
              key={job.key}
              onClick={() => onSelectJob(jobDetail)}
              style={{
                height: JOB_GRAPH_NODE_HEIGHT,
                left: node.x,
                top: node.y,
                width: JOB_GRAPH_NODE_WIDTH,
              }}
              type="button"
            >
              <RunStatusIcon state={job.state} />
              <span className="min-w-0 flex-1 truncate font-medium">{job.key}</span>
              {job.started_at_unix === null ? null : (
                <span className="text-xs text-muted-foreground">
                  <RunDuration end={job.completed_at_unix} start={job.started_at_unix} />
                </span>
              )}
            </button>
          )
        })}
      </div>
    </div>
  )
}
