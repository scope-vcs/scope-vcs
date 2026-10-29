import type { FileLineChangesResponse } from '@/api/types.generated'
import {
  abbreviatedCount,
  diffStatFilesLabel,
  diffStatHasLines,
  diffStatLabel,
  exactCount,
  fileLinesLabel,
  type DiffStat,
} from '@/lib/diff-stat'
import { cn } from '@/lib/utils'

export function DiffStatSummary({
  className,
  detail,
  stat,
}: {
  className?: string
  detail: 'compact' | 'exact'
  stat: DiffStat
}) {
  const label = diffStatLabel(stat)
  const exact = detail === 'exact'
  return (
    <span className={cn('whitespace-nowrap', className)} title={label}>
      <span aria-hidden="true">
        {diffStatHasLines(stat) ? (
          <>
            <LineCounts
              additions={stat.additions}
              deletions={stat.deletions}
              format={exact ? exactCount : abbreviatedCount}
            />
            {exact && ` · ${diffStatFilesLabel(stat.changed_files)}`}
          </>
        ) : diffStatFilesLabel(stat.changed_files)}
      </span>
      <span className="sr-only">{label}</span>
    </span>
  )
}

export function FileLineCounts({ lines }: { lines: FileLineChangesResponse }) {
  const label = fileLinesLabel(lines)
  return (
    <span className="whitespace-nowrap text-xs" title={label}>
      <span aria-hidden="true">
        {lines.kind === 'binary'
          ? <span className="text-muted-foreground">binary</span>
          : <LineCounts additions={lines.additions} deletions={lines.deletions} format={abbreviatedCount} />}
      </span>
      <span className="sr-only">{label}</span>
    </span>
  )
}

function LineCounts({
  additions,
  deletions,
  format,
}: {
  additions: number
  deletions: number
  format: (count: number) => string
}) {
  return (
    <span className="font-mono tabular-nums">
      <span className="text-success">+{format(additions)}</span>{' '}
      <span className="text-danger">−{format(deletions)}</span>
    </span>
  )
}
