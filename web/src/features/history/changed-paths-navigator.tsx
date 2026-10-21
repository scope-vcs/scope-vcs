import { NavigationSearch } from '@/components/navigation-search'
import { Button } from '@/components/ui/button'
import { BlockSkeleton } from '@/components/ui/block-skeleton'
import { TextSkeleton } from '@/components/ui/text-skeleton'
import { ToggleGroup, ToggleGroupItem } from '@/components/ui/toggle-group'
import { VisibilityBadge } from '@/components/visibility-badge'
import { cn } from '@/lib/utils'
import { ChevronRight } from 'lucide-react'
import type { ViewId } from '@/api/types.generated'
import { VisibilityTransitions } from './changed-path-context'
import type { ReactNode } from 'react'
import {
  canOpenChangedPath,
  groupChangedPaths,
  visibilitySummary,
  type ChangedPath,
  type ChangedPathFilter,
} from './changed-paths-model'

const FILTER_LABELS: Record<ChangedPathFilter, string> = {
  all: 'All',
  visibility: 'Visibility',
  Added: 'Added',
  Modified: 'Modified',
  Deleted: 'Deleted',
}

const KIND_CLASS = {
  Added: 'text-success-strong',
  Modified: 'text-warning-strong',
  Deleted: 'text-danger-strong',
} as const

const LIST_CLASS = 'lg:max-h-[min(70vh,720px)] lg:overflow-y-auto'
const TOOLBAR_CLASS = 'grid gap-2.5 border-b border-border px-(--app-inset) py-3'
const TOOLBAR_HEADING = <h2 className="text-sm font-medium">Changes</h2>

export function ChangedPathsToolbar({
  allOpen,
  counts,
  filter,
  onFilterChange,
  onOpenNavigation,
  onQueryChange,
  onToggleAll,
  query,
  visibility,
}: {
  allOpen: boolean
  counts: { filter: ChangedPathFilter; count: number }[]
  filter: ChangedPathFilter
  onFilterChange: (filter: ChangedPathFilter) => void
  onOpenNavigation: () => void
  onQueryChange: (query: string) => void
  onToggleAll: () => void
  query: string
  visibility: ReturnType<typeof visibilitySummary>
}) {
  return (
    <div className={TOOLBAR_CLASS}>
      {TOOLBAR_HEADING}
      <ToggleGroup
        aria-label="Show changes"
        className="flex-wrap"
        onValueChange={(value) => value && onFilterChange(value as ChangedPathFilter)}
        type="single"
        value={filter}
      >
        {counts.map(({ filter, count }) => (
          <ToggleGroupItem className="gap-1.5 text-xs" key={filter} value={filter}>
            {filter === 'visibility' && visibility && <VisibilityBadge compact visibility={visibility.label} />}
            {FILTER_LABELS[filter]}
            <span className="tabular-nums text-muted-foreground">{count}</span>
          </ToggleGroupItem>
        ))}
      </ToggleGroup>
      <div className="flex items-center gap-2">
        <NavigationSearch
          clearLabel="Clear path filter"
          label="Filter changed paths"
          onChange={onQueryChange}
          onOpen={onOpenNavigation}
          placeholder="Filter paths…"
          value={query}
        />
        {!query.trim() && (
          <Button onClick={onToggleAll} size="sm" type="button" variant="secondary">
            {allOpen ? 'Collapse all' : 'Expand all'}
          </Button>
        )}
      </div>
    </div>
  )
}

export function ChangedPathsList({
  countVisibility,
  onSelect,
  onToggleGroup,
  openGroups,
  paths,
  query,
  selectedPath,
  view,
}: {
  countVisibility: boolean
  onSelect: (path: ChangedPath) => void
  onToggleGroup: (group: string) => void
  openGroups: ReadonlySet<string>
  paths: ChangedPath[]
  query: string
  selectedPath: string | null
  view: ViewId
}) {
  const needle = query.trim().toLowerCase()
  const { root, groups } = groupChangedPaths(paths)
  const rows = (items: ChangedPath[], nested: boolean) => items.map((path, index) => (
    <li key={path.path}>
      {nested && path.directory && path.directory !== items[index - 1]?.directory && (
        <div className="break-all px-2 pt-2 pb-0.5 pl-8 font-mono text-[11px] text-muted-foreground">
          <Highlight needle={needle} text={`${path.directory}/`} />
        </div>
      )}
      <ChangedPathRow
        needle={needle}
        nested={nested}
        onSelect={onSelect}
        path={path}
        selected={path.path === selectedPath}
        view={view}
      />
    </li>
  ))

  if (paths.length === 0) {
    return <p className="px-2 py-4 text-xs text-muted-foreground">No changed paths match.</p>
  }
  return (
    <ul aria-label="Changed paths" className={cn('py-1', LIST_CLASS)}>
      {rows(root, false)}
      {groups.map((group) => {
        const open = Boolean(needle) || openGroups.has(group.name)
        const visibility = countVisibility ? visibilitySummary(group.paths) : null
        return (
          <li key={group.name}>
            <button
              aria-expanded={open}
              aria-label={[
                `${group.name}/`,
                `${group.paths.length} ${group.paths.length === 1 ? 'file' : 'files'}`,
                ...(visibility ? [`${visibility.count} visibility ${visibility.count === 1 ? 'change' : 'changes'}`] : []),
              ].join(', ')}
              className="flex min-h-8 w-full items-center gap-2 rounded-md px-2 text-left text-xs hover:bg-accent/50 disabled:cursor-default"
              disabled={Boolean(needle)}
              onClick={() => onToggleGroup(group.name)}
              type="button"
            >
              <ChevronRight
                aria-hidden
                className={cn('size-3.5 shrink-0 text-muted-foreground transition-transform', open && 'rotate-90')}
              />
              <span className="min-w-0 flex-1 truncate font-mono">
                <Highlight needle={needle} text={`${group.name}/`} />
              </span>
              {visibility && (
                <span className="flex items-center gap-1 tabular-nums text-muted-foreground">
                  <VisibilityBadge compact visibility={visibility.label} />
                  {visibility.count}
                </span>
              )}
              <span className="w-8 text-right tabular-nums text-muted-foreground">{group.paths.length}</span>
            </button>
            {open && <ul className="pb-1">{rows(group.paths, true)}</ul>}
          </li>
        )
      })}
    </ul>
  )
}

export function ChangedPathsToolbarSkeleton() {
  return (
    <div className={TOOLBAR_CLASS}>
      {TOOLBAR_HEADING}
      <BlockSkeleton className="h-[38px] w-48 rounded-lg" />
      <BlockSkeleton className="h-8 w-full" />
    </div>
  )
}

export function ChangedPathsSkeleton() {
  return (
    <div className="grid gap-2 py-3">
      {(['short', 'medium', 'short', 'long', 'medium'] as const).map((length, index) => (
        <TextSkeleton className="px-2" key={index} length={length} size="meta" />
      ))}
    </div>
  )
}

function ChangedPathRow({
  needle,
  nested,
  onSelect,
  path,
  selected,
  view,
}: {
  needle: string
  nested: boolean
  onSelect: (path: ChangedPath) => void
  path: ChangedPath
  selected: boolean
  view: ViewId
}) {
  const kind = path.file?.kind
  const openable = canOpenChangedPath(path)
  const latest = openable ? path.visibility.at(-1) : undefined
  const content: ReactNode = (
    <>
      <span
        aria-hidden
        className={cn('w-3 shrink-0 text-center font-mono text-[11px] font-semibold', kind ? KIND_CLASS[kind] : 'text-muted-foreground')}
      >
        {kind ? kind[0] : 'V'}
      </span>
      <span className={cn('min-w-0 flex-1 break-all font-mono', kind === 'Deleted' && 'text-muted-foreground line-through')}>
        <Highlight needle={needle} text={path.name} />
      </span>
      {latest && <VisibilityBadge compact visibility={latest.new_label} />}
    </>
  )
  const rowClass = cn(
    'flex min-h-7 w-full items-center gap-2 rounded-md py-1 pr-2 text-left text-xs',
    nested ? 'pl-8' : 'pl-2',
  )
  if (!openable) {
    return (
      <div className={cn(rowClass, 'flex-wrap text-muted-foreground')} title={`${path.path} · Visibility only`}>
        {content}
        <VisibilityTransitions changes={path.visibility} className="basis-full pl-5" view={view} />
      </div>
    )
  }
  return (
    <button
      aria-current={selected ? 'true' : undefined}
      className={cn(
        rowClass,
        'hover:bg-accent/50 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-[-2px] focus-visible:outline-ring',
        selected && 'bg-muted shadow-[inset_2px_0_0_0_var(--foreground)] hover:bg-muted',
      )}
      onClick={() => onSelect(path)}
      title={`${path.path} · ${kind ?? 'Visibility only'}`}
      type="button"
    >
      {content}
    </button>
  )
}

function Highlight({ needle, text }: { needle: string; text: string }) {
  const start = needle ? text.toLowerCase().indexOf(needle) : -1
  if (start === -1) return text
  const end = start + needle.length
  return (
    <>
      {text.slice(0, start)}
      <mark className="rounded-sm bg-warning-soft text-foreground ring-1 ring-warning-border">{text.slice(start, end)}</mark>
      {text.slice(end)}
    </>
  )
}
