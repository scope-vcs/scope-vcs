import type { RepoParams } from '@/api/types'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ChevronRight } from 'lucide-react'
import { RunStatusIcon } from '../runs/run-status-icon'
import { type RequestCheckRow, requestCheckGroups } from './request-check-rows'

const ROW_CLASS = 'group flex min-w-0 items-center gap-2 rounded-md py-1.5 text-[13px]'

export function RequestCheckWorkflows({ id, params, rows }: {
  id: string
  params: RepoParams
  rows: RequestCheckRow[]
}) {
  if (!rows.length) return null
  const groups = requestCheckGroups(rows)
  const named = groups.length > 1
  return (
    <div className="mt-2 grid min-w-0 gap-2" id={id}>
      {groups.map((group) => group.kind === 'native' ? (
        <ResultRow key={group.key} params={params} row={group.row} />
      ) : (
        <section aria-label={group.kind === 'workflow' ? `${group.name} workflow` : 'Other results'} key={group.key}>
          {named ? (
            <h3 className="min-w-0 break-words text-xs text-muted-foreground">
              {group.kind === 'workflow' ? (
                <Link className="hover:text-foreground hover:underline" params={{ ...params, runId: group.runId }} to="/$owner/$repo/runs/$runId">
                  {group.name}
                </Link>
              ) : 'Other results'}
            </h3>
          ) : null}
          <ul aria-label={group.kind === 'workflow' ? `${group.name} jobs` : 'Results without a workflow'}>
            {group.jobs.map((row) => <li key={row.key}><ResultRow params={params} row={row} /></li>)}
          </ul>
        </section>
      ))}
    </div>
  )
}

function ResultRow({ params, row }: { params: RepoParams; row: RequestCheckRow }) {
  const content = (
    <>
      <RunStatusIcon state={row.state} />
      <span className="min-w-0 flex-1" title={row.name}>
        <span className={cn('block break-words', row.tone === 'inert' && 'text-muted-foreground')}>{row.leaf}</span>
        {row.parents.length ? <span className="block break-words text-xs text-muted-foreground">{row.parents.join(' / ')}</span> : null}
      </span>
      {row.reason ? <span className="shrink-0 text-xs text-danger-strong">{row.reason}</span> : null}
      <ChevronRight aria-hidden="true" className={cn('size-3 shrink-0', row.run ? 'opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100' : 'invisible')} />
    </>
  )
  return row.run ? (
    <Link className={cn(ROW_CLASS, '-mx-1.5 px-1.5 hover:bg-accent focus-visible:bg-accent')} hash={row.run.hash} params={{ ...params, runId: row.run.id }} to="/$owner/$repo/runs/$runId">
      {content}
    </Link>
  ) : <div className={ROW_CLASS}>{content}</div>
}
