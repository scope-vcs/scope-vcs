import type { HistoryFeed } from '@/api/types.generated'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { HISTORY_FEED_ROW_CLASS, HISTORY_FEEDS } from './history-feeds'

export function HistoryFeedToggle({
  feed,
  onSelect,
}: {
  feed: HistoryFeed
  onSelect: (feed: HistoryFeed) => void
}) {
  return (
    <div className={HISTORY_FEED_ROW_CLASS}>
      <ToggleGroup
        aria-label="History activity"
        onValueChange={(value) => {
          if (value === 'updates' || value === 'all') onSelect(value)
        }}
        type="single"
        value={feed}
      >
        {HISTORY_FEEDS.map(({ label, value }) => (
          <ToggleGroupItem key={value} value={value}>{label}</ToggleGroupItem>
        ))}
      </ToggleGroup>
    </div>
  )
}
