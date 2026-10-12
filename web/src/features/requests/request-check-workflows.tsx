import type { RepoParams } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ChevronRight, Workflow } from 'lucide-react'
import { RunStatusIcon } from '../runs/run-status-icon'
import { type RequestCheckRow, requestCheckGroups } from './request-check-rows'

const ROW_CLASS = 'group flex min-w-0 items-center gap-2 rounded-md py-1.5 text-[13px]'

export function RequestCheckWorkflows({ id, params, rows }: {
  id: string
  params: RepoParams
  rows: RequestCheckRow[]
}) {
  if (!rows.length) return null
  return (
    <div className="mt-4 grid min-w-0 gap-4" id={id}>
      {requestCheckGroups(rows).map((group) => (
        <section aria-label={group.kind === 'unassigned' ? 'Other results' : `${group.kind === 'native' ? group.row.name : group.name} workflow`} key={group.key}>
          {group.kind === 'native' ? (
            <h3>
              <ResultRow params={params} row={group.row} workflow />
            </h3>
          ) : (
            <>
              <h3 className="flex min-w-0 items-center gap-2 text-[13px] font-medium">
                {group.kind === 'workflow' ? (
                  <>
                    <Workflow aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" />
                    <Link className="min-w-0 break-words hover:underline" params={{ ...params, runId: group.runId }} to="/$owner/$repo/runs/$runId">
                      {group.name}
                    </Link>
                    <span className="shrink-0 font-mono text-[10px] font-normal uppercase tracking-wide text-muted-foreground">Workflow</span>
                  </>
                ) : 'Other results'}
              </h3>
              <ul aria-label={group.kind === 'workflow' ? `${group.name} jobs` : 'Results without a workflow'} className="ml-[7px] mt-1 border-l border-border pl-4">
                {group.jobs.map((row) => <li key={row.key}><ResultRow params={params} row={row} /></li>)}
              </ul>
            </>
          )}
        </section>
      ))}
    </div>
  )
}

function ResultRow({ params, row, workflow = false }: {
  params: RepoParams
  row: RequestCheckRow
  workflow?: boolean
}) {
  const content = (
    <>
      {workflow ? <Workflow aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground" /> : <RunStatusIcon state={row.state} />}
      <span className="min-w-0 flex-1" title={row.name}>
        <span className={cn('block break-words', workflow && 'font-medium', row.tone === 'inert' && 'text-muted-foreground')}>
          {workflow ? row.name : row.leaf}
        </span>
        {!workflow && row.parents.length ? <span className="block break-words text-[11px] text-muted-foreground">{row.parents.join(' / ')}</span> : null}
      </span>
      {workflow ? <span className="font-mono text-[10px] uppercase tracking-wide text-muted-foreground">Workflow</span> : null}
      {workflow ? <RunStatusIcon state={row.state} /> : null}
      {row.tone === 'success' || row.tone === 'inert' ? null : (
        <span className={cn('shrink-0 font-mono text-xs', row.tone === 'danger' ? 'text-danger-strong' : 'text-muted-foreground')}>{row.label}</span>
      )}
      <ChevronRight aria-hidden="true" className={cn('size-3 shrink-0', row.run ? 'opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100' : 'invisible')} />
    </>
  )
  return row.run ? (
    <Link className={cn(ROW_CLASS, '-mx-1.5 px-1.5 hover:bg-accent focus-visible:bg-accent')} hash={row.run.hash} params={{ ...params, runId: row.run.id }} to="/$owner/$repo/runs/$runId">
      {content}
    </Link>
  ) : <div className={ROW_CLASS}>{content}</div>
}
