import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ViewId } from '@/api/types.generated'
import type { RepositoryAccessResponse } from '@/api/types.generated'
import { mayReadView, readableViews } from '@/api/repo-views'
import { MenuListPanel } from '@/components/menu-list-panel'
import { Popover } from '@/components/ui/popover'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { ChevronDown, History } from 'lucide-react'
import { useState } from 'react'
import { HistoryFeedList } from './history-entry-list'
import { useHistoryFeed } from './history-feed'
import { updateViewSearch } from './update-search'

const FEEDS: { value: HistoryFeed; label: string; empty: string }[] = [
  { value: 'all', label: 'All', empty: 'No history yet.' },
  { value: 'updates', label: 'Pushes & merges', empty: 'No pushes or merges yet.' },
  { value: 'visibility', label: 'Visibility', empty: 'No visibility changes yet.' },
]

export function HistoryMenu({
  access,
  initialFeed = 'all',
  label = 'History',
  params,
}: {
  access: RepositoryAccessResponse
  initialFeed?: HistoryFeed
  label?: string
  params: RepoParams
}) {
  const defaultView = access.view
  const [feed, setFeed] = useState<HistoryFeed>(initialFeed)
  const [view, setView] = useState<ViewId>(defaultView)
  const selectedView = mayReadView(access, view) ? view : defaultView

  return (
    <Popover
      className="w-[min(30rem,calc(100vw-3rem))] p-0"
      label="Repository history"
      panel={(close) => (
        <HistoryMenuPanel
          view={selectedView}
          access={access}
          defaultView={defaultView}
          feed={feed}
          onNavigate={close}
          onSelectView={setView}
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
  access,
  defaultView,
  feed,
  onNavigate,
  onSelectView,
  onSelectFeed,
  params,
}: {
  view: ViewId
  access: RepositoryAccessResponse
  defaultView: ViewId
  feed: HistoryFeed
  onNavigate: () => void
  onSelectView: (view: ViewId) => void
  onSelectFeed: (feed: HistoryFeed) => void
  params: RepoParams
}) {
  const history = useHistoryFeed({ view, feed, params })
  const empty = FEEDS.find((option) => option.value === feed)?.empty ?? ''
  const options = readableViews(access)

  return (
    <MenuListPanel
      controls={(
        <>
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
          {options.length > 1 ? (
            <ToggleGroup
              aria-label="Viewing as"
              onValueChange={(value) => {
                if (value) onSelectView(value as ViewId)
              }}
              type="single"
              value={view}
            >
              {options.map((option) => (
                <ToggleGroupItem key={option.id} value={option.id}>{option.name}</ToggleGroupItem>
              ))}
            </ToggleGroup>
          ) : null}
        </>
      )}
    >
      <HistoryFeedList
        empty={empty}
        history={history}
        onNavigate={onNavigate}
        params={params}
        search={updateViewSearch(view, defaultView)}
      />
    </MenuListPanel>
  )
}
