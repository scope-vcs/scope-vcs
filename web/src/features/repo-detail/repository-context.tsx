import type { RepoContent } from '@/api/types'
import type { RepoSummaryResponse } from '@/api/types.generated'
import { Globe2, LockKeyhole, ExternalLink } from 'lucide-react'
import { RepositoryResourcesMenu } from './repository-resources-menu'

export function RepositoryContext({
  content,
  contentLoading,
  onSelectFilePath,
  repo,
}: {
  content: RepoContent | null
  contentLoading: boolean
  onSelectFilePath: (path: string) => void
  repo: RepoSummaryResponse
}) {
  const includesPrivate = content?.files.some((file) => file.visibility === 'Private') ?? false
  const viewLabel = includesPrivate
    ? 'Includes private files'
    : repo.access.can_read_private_files ? 'Full view' : 'Public view'
  return (
    <div className="min-w-0">
      {repo.description && <p className="mb-2 max-w-[75ch] break-words text-sm leading-5 text-foreground">{repo.description}</p>}
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-muted-foreground">
        <span className="inline-flex items-center gap-1.5">
          {includesPrivate ? <LockKeyhole aria-hidden="true" className="size-3.5" /> : <Globe2 aria-hidden="true" className="size-3.5" />}
          {viewLabel}
        </span>
        {content ? (
          <span>{content.files.length} {content.files.length === 1 ? 'file' : 'files'}</span>
        ) : !contentLoading ? <span>Files unavailable</span> : null}
        {repo.website_url && (
          <a
            className="inline-flex min-w-0 max-w-full items-center gap-1 rounded hover:text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring"
            href={repo.website_url}
            rel="noopener noreferrer"
            target="_blank"
            title={repo.website_url}
          >
            <span className="truncate">{websiteLabel(repo.website_url)}</span>
            <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
          </a>
        )}
        {content && <RepositoryResourcesMenu files={content.files} onSelectFilePath={onSelectFilePath} />}
      </div>
    </div>
  )
}

function websiteLabel(url: string) {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '')
}
