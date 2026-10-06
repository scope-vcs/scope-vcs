import type { RepoViews } from '@/api/repo-views'
import type { ViewDefinition } from '@/api/types.generated'
import { SectionRow, SectionRows } from '../../components/section-rows'
import { Layers } from 'lucide-react'

export function RepositoryViewsSection({ views }: { views: RepoViews }) {
  return (
    <SectionRows>
      <SectionRow
        description="Every file carries one view label. A view shows its own files and the files of the views it includes."
        icon={<Layers className="size-4" />}
        id="views"
        title="Views"
      >
        <ul aria-label="Repository views" className="divide-y divide-border text-sm">
          {views.definitions.map((view) => (
            <li className="grid gap-0.5 py-2 first:pt-0 sm:flex sm:items-baseline sm:justify-between sm:gap-4" key={view.id}>
              <span className="min-w-0 break-words">
                <span className="font-medium">{view.name}</span>
                {' '}
                <span className="font-mono text-xs text-muted-foreground">{view.id}</span>
              </span>
              <span className="shrink-0 text-xs text-muted-foreground">
                {includesText(views, view)} · {readersText(view)}
              </span>
            </li>
          ))}
        </ul>
        <p className="mt-3 text-xs leading-5 text-muted-foreground">
          Change views with <code className="font-mono">scope view</code> in the CLI, then publish them with{' '}
          <code className="font-mono">scope push --main</code>.
        </p>
      </SectionRow>
    </SectionRows>
  )
}

function includesText(views: RepoViews, view: ViewDefinition) {
  const included = views.includedNames(view.id)
  if (included === 'all') return 'Includes every view'
  return included.length > 0 ? `Includes ${included.join(', ')}` : 'Its own files only'
}

function readersText(view: ViewDefinition) {
  return view.readers === 'anyone' ? 'Readable by anyone' : 'Readable by assigned members'
}
