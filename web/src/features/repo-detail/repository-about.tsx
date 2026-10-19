import type { RepoSummaryResponse } from '@/api/types.generated'
import { ExternalLink } from 'lucide-react'

export function hasRepositoryAbout(repo: RepoSummaryResponse) {
  return Boolean(repo.description || repo.website_url)
}

export function RepositoryAbout({ repo }: { repo: RepoSummaryResponse }) {
  return (
    <p className="flex min-w-0 items-center gap-x-3 text-xs">
      {repo.description && <span className="min-w-0 truncate" title={repo.description}>{repo.description}</span>}
      {repo.website_url && (
        <a
          className="inline-flex min-w-0 max-w-1/2 shrink-0 items-center gap-1 rounded hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          href={repo.website_url}
          rel="noopener noreferrer"
          target="_blank"
          title={repo.website_url}
        >
          <span className="truncate">{websiteLabel(repo.website_url)}</span>
          <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
        </a>
      )}
    </p>
  )
}

function websiteLabel(url: string) {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '')
}
