import type { ReactNode } from 'react'
import { runJobShortName } from './run-job-tree-model'
import { RunStatusIcon } from './run-status-icon'

export function RunJobHeading({
  detail,
  name,
  state,
  tools,
}: {
  detail: ReactNode
  name: string
  state: string
  tools: ReactNode
}) {
  return (
    <div className="flex min-h-12 flex-none flex-wrap items-center gap-x-2.5 gap-y-1.5 border-b border-border py-2 pl-4 pr-3">
      <span className="flex min-w-0 items-center gap-2.5 max-lg:sr-only">
        <RunStatusIcon state={state} />
        <h2 className="min-w-0 break-words text-[15px] font-semibold leading-5" title={name}>{runJobShortName(name)}</h2>
      </span>
      {detail ? <span className="min-w-0 text-xs text-muted-foreground">{detail}</span> : null}
      {tools ? <div className="ml-auto flex min-w-0 items-center gap-1 max-sm:w-full">{tools}</div> : null}
    </div>
  )
}
