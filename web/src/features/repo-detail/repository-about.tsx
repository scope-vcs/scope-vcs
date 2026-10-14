import type { RepoFileResponse, RepoSummaryResponse } from '@/api/types.generated'
import { ExternalLink } from 'lucide-react'
import { useId } from 'react'
import { repositoryResources } from './repository-file-navigation'

const LINK_CLASS = 'flex w-full min-w-0 items-center gap-1.5 rounded px-2 py-1 text-left text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring'

export function RepositoryAbout({
  files,
  onSelectFilePath,
  repo,
}: {
  files: RepoFileResponse[]
  onSelectFilePath: (path: string) => void
  repo: RepoSummaryResponse
}) {
  const headingId = useId()
  const resources = repositoryResources(files)
  if (!repo.description && !repo.website_url && !resources.length) return null
  return (
    <section aria-labelledby={headingId} className="mt-4 border-t border-border pt-3 text-xs">
      <h2 className="mb-1 px-2 text-muted-foreground" id={headingId}>About</h2>
      {repo.description && <p className="mb-1 break-words px-2 leading-5 text-foreground">{repo.description}</p>}
      <ul>
        {repo.website_url && (
          <li>
            <a
              className={LINK_CLASS}
              href={repo.website_url}
              rel="noopener noreferrer"
              target="_blank"
              title={repo.website_url}
            >
              <span className="truncate">{websiteLabel(repo.website_url)}</span>
              <ExternalLink aria-hidden="true" className="size-3 shrink-0" />
            </a>
          </li>
        )}
        {resources.map(({ label, path }) => (
          <li key={label}>
            <button className={LINK_CLASS} onClick={() => onSelectFilePath(path)} type="button">
              {label}
            </button>
          </li>
        ))}
      </ul>
    </section>
  )
}

function websiteLabel(url: string) {
  return url.replace(/^https?:\/\//, '').replace(/\/$/, '')
}
