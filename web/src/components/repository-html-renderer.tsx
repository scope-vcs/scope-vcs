import { useThemeType } from '@/lib/use-theme-type'
import { useMemo } from 'react'
import { repositoryHtmlDocument } from './repository-html'
import { PersistentRepositoryHtmlPreview } from './repository-html-preview-store'

export type RepositoryHtmlMode = 'preview' | 'source'

export function RepositoryHtmlRenderer({
  identity,
  path,
  mode,
  source,
}: {
  identity: string
  path: string
  mode: RepositoryHtmlMode
  source: string
}) {
  const theme = useThemeType()
  const document = useMemo(
    () => repositoryHtmlDocument(source, theme),
    [source, theme],
  )
  const displayPath = path.replace(/^\/+/, '')

  return (
    <div className="min-w-0">
      {mode === 'preview' ? (
        <PersistentRepositoryHtmlPreview
          className="h-[calc(100dvh-var(--app-chrome)-84px)] min-h-[32rem] max-h-[70rem] w-full border-0 bg-background"
          identity={`${identity}:${theme}`}
          srcDoc={document}
          title={`${displayPath} preview`}
        />
      ) : (
        <pre className="min-h-[32rem] overflow-x-auto bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
          <code>{source}</code>
        </pre>
      )}
    </div>
  )
}
