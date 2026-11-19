import { useThemeType } from '@/lib/use-theme-type'
import { displayRouteFilePath } from '@/lib/route-file'
import { useMemo } from 'react'
import { CodeSourceView } from '@/features/code-render/prerendered-code'
import { repositoryHtmlDocument } from './repository-html'
import { PersistentRepositoryHtmlPreview } from './repository-html-preview-store'

export type RepositoryHtmlMode = 'preview' | 'source'

export function RepositoryHtmlRenderer({
  highlightedHtml,
  identity,
  path,
  mode,
  source,
}: {
  highlightedHtml: string | null
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
  const previewIdentity = `${identity}:${theme}`

  return (
    <div className="min-w-0">
      {mode === 'preview' ? (
        <PersistentRepositoryHtmlPreview
          className="h-[calc(100dvh-var(--app-chrome)-84px)] min-h-[32rem] max-h-[70rem] w-full border-0 bg-background"
          identity={previewIdentity}
          key={previewIdentity}
          srcDoc={document}
          title={`${displayRouteFilePath(path)} preview`}
        />
      ) : (
        <div className="min-h-[32rem]">
          <CodeSourceView highlightedHtml={highlightedHtml} source={source} />
        </div>
      )}
    </div>
  )
}
