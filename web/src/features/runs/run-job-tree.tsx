import { cn } from '@/lib/utils'
import { ChevronDown, Search } from 'lucide-react'
import { useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { RunDuration } from './run-duration'
import { runJobPanelId } from './run-job-ids'
import {
  buildRunJobTree,
  failedJobCount,
  filterRunJobTree,
  runJobGroupOpensByDefault,
  runJobShortName,
  type RunJobGroup,
  type RunJobNode,
  type RunJobSummary,
} from './run-job-tree-model'
import { RunStatusIcon } from './run-status-icon'

const FILTER_THRESHOLD = 10

type TreeProps = {
  onSelect: (key: string) => void
  selectedKey: string | null
}

export function RunJobTree({
  footer,
  jobs,
  onSelect,
  selectedKey,
}: TreeProps & { footer?: ReactNode; jobs: readonly RunJobSummary[] }) {
  const listId = useId()
  const pickerRef = useRef<HTMLButtonElement>(null)
  const [pickerOpen, setPickerOpen] = useState(false)
  const [filter, setFilter] = useState('')
  const [groupOpen, setGroupOpen] = useState<Readonly<Record<string, boolean>>>({})
  const tree = useMemo(() => buildRunJobTree(jobs), [jobs])
  const shown = useMemo(() => filterRunJobTree(tree, filter), [filter, tree])
  const selected = jobs.find((job) => job.key === selectedKey) ?? null
  const filtering = filter.trim() !== ''

  function select(key: string) {
    if (pickerOpen) pickerRef.current?.focus()
    setPickerOpen(false)
    onSelect(key)
  }

  return (
    <nav
      aria-label="Jobs"
      className="flex min-w-0 flex-wrap items-center border-b border-border lg:flex-col lg:flex-nowrap lg:items-stretch lg:overflow-y-auto lg:border-b-0 lg:border-r lg:py-2"
    >
      <button
        aria-controls={listId}
        aria-expanded={pickerOpen}
        className="m-2 flex h-9 min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-card px-3 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring lg:hidden"
        onClick={() => setPickerOpen((open) => !open)}
        ref={pickerRef}
        type="button"
      >
        {selected ? <RunStatusIcon state={selected.state} /> : null}
        <span className="min-w-0 flex-1 truncate font-medium">{selected ? runJobShortName(selected.name) : 'Choose a job'}</span>
        <span className="shrink-0 text-xs text-muted-foreground">{jobs.length} {jobs.length === 1 ? 'job' : 'jobs'}</span>
        <ChevronDown aria-hidden="true" className={cn('size-4 shrink-0 text-muted-foreground transition-transform', pickerOpen && 'rotate-180')} />
      </button>
      {footer ? <div className="shrink-0 lg:order-last">{footer}</div> : null}
      <div className={cn('basis-full pb-2 lg:block lg:basis-auto lg:pb-0', pickerOpen ? 'block' : 'hidden')} id={listId}>
        {jobs.length > FILTER_THRESHOLD ? (
          <label className="mx-2.5 mb-2 flex h-8 items-center gap-2 rounded-md border border-border bg-card px-2.5 text-sm focus-within:border-ring focus-within:outline focus-within:outline-2 focus-within:outline-offset-1 focus-within:outline-ring">
            <Search aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
            <input
              aria-label="Filter jobs"
              className="min-w-0 flex-1 bg-transparent outline-none placeholder:text-muted-foreground"
              onChange={(event) => setFilter(event.target.value)}
              placeholder={`Filter ${jobs.length} jobs`}
              type="search"
              value={filter}
            />
          </label>
        ) : null}
        {filtering && shown.length === 0 ? (
          <p className="px-3 py-2 text-sm text-muted-foreground">No jobs match.</p>
        ) : (
          <JobNodes
            depth={0}
            isOpen={(group) => filtering || (groupOpen[group.path] ?? runJobGroupOpensByDefault(group, selectedKey))}
            nodes={shown}
            onSelect={select}
            onToggle={(group, open) => setGroupOpen((current) => ({ ...current, [group.path]: !open }))}
            selectedKey={selectedKey}
          />
        )}
      </div>
    </nav>
  )
}

function JobNodes({
  depth,
  isOpen,
  nodes,
  onSelect,
  onToggle,
  selectedKey,
}: TreeProps & {
  depth: number
  isOpen: (group: RunJobGroup) => boolean
  nodes: readonly RunJobNode[]
  onToggle: (group: RunJobGroup, open: boolean) => void
}) {
  const indent = { '--tree-indent': `${0.75 + depth}rem` } as CSSProperties
  return (
    <ul>
      {nodes.map((node) => {
        if (node.kind === 'job') {
          const { job } = node
          const selected = job.key === selectedKey
          return (
            <li key={job.key}>
              <button
                aria-controls={runJobPanelId(job.key)}
                aria-pressed={selected}
                className={cn(
                  'relative grid w-full grid-cols-[0.875rem_minmax(0,1fr)_auto] items-start gap-2 py-1.5 pl-(--tree-indent) pr-3 text-left text-[13.5px] leading-[18px] outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring',
                  selected && 'bg-muted font-medium before:absolute before:inset-y-1.5 before:left-0 before:w-0.5 before:rounded-full before:bg-foreground',
                )}
                onClick={() => onSelect(job.key)}
                style={indent}
                title={job.name}
                type="button"
              >
                <span className="mt-0.5 flex"><RunStatusIcon state={job.state} /></span>
                <span className="min-w-0 break-words [overflow-wrap:anywhere]">{node.label}</span>
                {job.startedAt === null ? null : (
                  <span className="text-xs font-normal text-muted-foreground">
                    <RunDuration end={job.completedAt} start={job.startedAt} />
                  </span>
                )}
              </button>
            </li>
          )
        }
        const open = isOpen(node)
        const failed = failedJobCount(node.jobs)
        return (
          <li key={node.path}>
            <button
              aria-expanded={open}
              className="flex w-full items-center gap-1.5 py-1.5 pl-(--tree-indent) pr-3 text-left text-xs font-semibold text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
              onClick={() => onToggle(node, open)}
              style={indent}
              type="button"
            >
              <ChevronDown aria-hidden="true" className={cn('size-3.5 shrink-0 transition-transform', !open && '-rotate-90')} />
              <span className="min-w-0 flex-1 break-words">{node.label}</span>
              <span className="shrink-0 font-normal tabular-nums">
                {failed > 0 ? <span className="text-danger-strong">{failed} failed · </span> : null}
                {node.jobs.length}
              </span>
            </button>
            {open ? (
              <JobNodes
                depth={depth + 1}
                isOpen={isOpen}
                nodes={node.children}
                onSelect={onSelect}
                onToggle={onToggle}
                selectedKey={selectedKey}
              />
            ) : null}
          </li>
        )
      })}
    </ul>
  )
}
