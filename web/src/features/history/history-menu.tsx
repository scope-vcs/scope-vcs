import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ViewId } from '@/api/types.generated'
import { MenuListPanel } from '@/components/menu-list-panel'
import { Popover } from '@/components/ui/popover'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { useViewingAs } from '@/features/repo-detail/use-viewing-as'
import { ChevronDown, History } from 'lucide-react'
import { useState } from 'react'
import { HistoryFeedList } from './history-entry-list'
import { useHistoryFeed } from './history-feed'

const FEEDS: { value: HistoryFeed; label: string; empty: string }[] = [
  { value: 'all', label: 'All', empty: 'No history yet.' },
  { value: 'updates', label: 'Pushes & merges', empty: 'No pushes or merges yet.' },
  { value: 'visibility', label: 'Visibility', empty: 'No visibility changes yet.' },
]

export function HistoryMenu({
  initialFeed = 'all',
  label = 'History',
  params,
}: {
  initialFeed?: HistoryFeed
  label?: string
  params: RepoParams
}) {
  const { view } = useViewingAs()
  const [feed, setFeed] = useState<HistoryFeed>(initialFeed)

  return (
    <Popover
      className="w-[min(30rem,calc(100vw-3rem))] p-0"
      label="Repository history"
      panel={(close) => (
        <HistoryMenuPanel
          view={view}
          feed={feed}
          onNavigate={close}
          onSelectFeed={setFeed}
          params={params}
        />
      )}
      trigger={(props) => (
        <button
          className="-my-1 flex shrink-0 cursor-pointer items-center gap-1.5 rounded px-2 py-1 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring aria-expanded:bg-muted aria-expanded:text-foreground"
          type="button"
          {...props}
        >
          <History aria-hidden="true" className="size-3.5" /> {label}
          <ChevronDown aria-hidden="true" className="size-3.5" />
        </button>
      )}
    />
  )
}

function HistoryMenuPanel({
  view,
  feed,
  onNavigate,
  onSelectFeed,
  params,
}: {
  view: ViewId
  feed: HistoryFeed
  onNavigate: () => void
  onSelectFeed: (feed: HistoryFeed) => void
  params: RepoParams
}) {
  const history = useHistoryFeed({ view, feed, params })
  const empty = FEEDS.find((option) => option.value === feed)?.empty ?? ''

  return (
    <MenuListPanel
      controls={(
        <ToggleGroup
          aria-label="History filter"
          onValueChange={(value) => {
            if (value) onSelectFeed(value as HistoryFeed)
          }}
          type="single"
          value={feed}
        >
          {FEEDS.map((option) => (
            <ToggleGroupItem key={option.value} value={option.value}>{option.label}</ToggleGroupItem>
          ))}
        </ToggleGroup>
      )}
    >
      <HistoryFeedList
        empty={empty}
        history={history}
        onNavigate={onNavigate}
        params={params}
      />
    </MenuListPanel>
  )
}
