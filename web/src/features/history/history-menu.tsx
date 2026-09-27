import type { RepoParams } from '@/api/types'
import type { HistoryFeed, ProjectionPreviewAudience } from '@/api/types.generated'
import { MenuListPanel } from '@/components/menu-list-panel'
import { Popover } from '@/components/ui/popover'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { ChevronDown, History } from 'lucide-react'
import { useState } from 'react'
import { HistoryFeedList } from './history-entry-list'
import { defaultHistoryAudience, useHistoryFeed } from './history-feed'
import { updateAudienceSearch } from './update-search'

const FEEDS: { value: HistoryFeed; label: string; empty: string }[] = [
  { value: 'all', label: 'All', empty: 'No history yet.' },
  { value: 'updates', label: 'Pushes & merges', empty: 'No pushes or merges yet.' },
  { value: 'visibility', label: 'Visibility', empty: 'No visibility changes yet.' },
]

export function HistoryMenu({
  canReadPrivateFiles,
  params,
}: {
  canReadPrivateFiles: boolean
  params: RepoParams
}) {
  const defaultAudience = defaultHistoryAudience(canReadPrivateFiles)
  // Held outside the panel so a reopened menu keeps the reader's last filter.
  const [feed, setFeed] = useState<HistoryFeed>('all')
  const [audience, setAudience] = useState<ProjectionPreviewAudience>(defaultAudience)

  return (
    <Popover
      className="w-[min(30rem,calc(100vw-2rem))] p-0"
      label="Repository history"
      panel={(close) => (
        <HistoryMenuPanel
          audience={audience}
          canReadPrivateFiles={canReadPrivateFiles}
          defaultAudience={defaultAudience}
          feed={feed}
          onNavigate={close}
          onSelectAudience={setAudience}
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
          <History aria-hidden="true" className="size-3.5" /> History
          <ChevronDown aria-hidden="true" className="size-3.5" />
        </button>
      )}
    />
  )
}

function HistoryMenuPanel({
  audience,
  canReadPrivateFiles,
  defaultAudience,
  feed,
  onNavigate,
  onSelectAudience,
  onSelectFeed,
  params,
}: {
  audience: ProjectionPreviewAudience
  canReadPrivateFiles: boolean
  defaultAudience: ProjectionPreviewAudience
  feed: HistoryFeed
  onNavigate: () => void
  onSelectAudience: (audience: ProjectionPreviewAudience) => void
  onSelectFeed: (feed: HistoryFeed) => void
  params: RepoParams
}) {
  const history = useHistoryFeed({ audience, feed, params })
  const empty = FEEDS.find((option) => option.value === feed)?.empty ?? ''

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
          {canReadPrivateFiles ? (
            <ToggleGroup
              aria-label="Viewing as"
              onValueChange={(value) => {
                if (value) onSelectAudience(value as ProjectionPreviewAudience)
              }}
              type="single"
              value={audience}
            >
              <ToggleGroupItem value="private">Private</ToggleGroupItem>
              <ToggleGroupItem value="public">Public</ToggleGroupItem>
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
        search={updateAudienceSearch(audience, defaultAudience)}
      />
    </MenuListPanel>
  )
}
