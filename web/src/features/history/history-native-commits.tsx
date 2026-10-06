import type { HistoryEntryDetailResponse } from '@/api/types.generated'
import { useRepoViews } from '@/features/repo-detail/repo-layout-context'
import { historyNativeCommitRows, historyNativeCommitsHeading } from './history-native-commits-model'

export function NativeCommits({ detail }: { detail: HistoryEntryDetailResponse }) {
  const views = useRepoViews()
  const rows = historyNativeCommitRows(detail)
  if (rows.length === 0) return null
  const heading = historyNativeCommitsHeading(detail, views)
  return (
    <section aria-label={heading} className="border-b border-border">
      <h2 className="px-5 pt-3 pb-1 text-sm font-medium sm:px-6">{heading}</h2>
      <ol className="divide-y divide-border">
        {rows.map((row) => (
          <li className="grid grid-cols-[auto_minmax(0,1fr)] items-baseline gap-x-3 px-5 py-2 text-xs sm:px-6" key={row.oid}>
            <span className="select-all font-mono text-[11px] text-muted-foreground" title={row.oid}>
              {row.shortOid}
            </span>
            <span className="flex min-w-0 flex-wrap items-baseline justify-between gap-x-3 gap-y-0.5">
              <span className="min-w-0 break-words text-foreground">{row.title}</span>
              <span className="min-w-0 break-words text-muted-foreground">
                {row.author} · {row.fileCount}
              </span>
            </span>
          </li>
        ))}
      </ol>
    </section>
  )
}
