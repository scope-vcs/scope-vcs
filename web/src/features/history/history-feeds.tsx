import type { HistoryFeed } from '@/api/types.generated'
import { buttonVariants } from '@/components/ui/button-variants'
import { TOGGLE_GROUP_CLASS, TOGGLE_GROUP_ITEM_CLASS } from '@/components/ui/toggle-group-variants'
import { cn } from '@/lib/utils'

export const HISTORY_FEEDS: { label: string; value: HistoryFeed }[] = [
  { label: 'Pushes & merges', value: 'updates' },
  { label: 'All activity', value: 'all' },
]

export const HISTORY_FEED_ROW_CLASS = 'border-b border-border px-5 py-3 sm:px-6'

/** The feed toggle drawn statically while history loads, without Radix. */
export function HistoryFeedTogglePending({ feed }: { feed: HistoryFeed }) {
  return (
    <div className={HISTORY_FEED_ROW_CLASS}>
      <div aria-hidden="true" className={TOGGLE_GROUP_CLASS}>
        {HISTORY_FEEDS.map(({ label, value }) => (
          <span
            className={cn(buttonVariants({ size: 'sm', variant: 'ghost' }), TOGGLE_GROUP_ITEM_CLASS)}
            data-state={value === feed ? 'on' : 'off'}
            key={value}
          >
            {label}
          </span>
        ))}
      </div>
    </div>
  )
}
