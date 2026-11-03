import type { RepoParams } from '@/api/types'
import type { RepoFileContentResponse } from '@/api/types.generated'
import { PanelState } from '@/components/empty-state'
import { isRepositoryHtmlPath } from '@/components/repository-html'
import { RepositoryHtmlRenderer, type RepositoryHtmlMode } from '@/components/repository-html-renderer'
import { isRepositoryMarkdownPath } from '@/components/repository-markdown'
import { RepositoryMarkdownRenderer } from '@/components/repository-markdown-renderer'
import { formatBytes } from '@/lib/format-bytes'
import { FileQuestion } from 'lucide-react'

export function RepositoryFileContent({
  file,
  htmlMode,
  params,
}: {
  file: RepoFileContentResponse
  htmlMode: RepositoryHtmlMode
  params: RepoParams
}) {
  if (file.content.kind !== 'text') {
    return (
      <PanelState>
        <FileQuestion className="size-5" />
        <span>
          Binary file not rendered ·{' '}
          {formatBytes(file.content.size_bytes)} ·{' '}
          {file.content.oid.slice(0, 12)}
        </span>
      </PanelState>
    )
  }

  if (isRepositoryMarkdownPath(file.path)) {
    return (
      <RepositoryMarkdownRenderer
        repository={{ ...params, markdownPath: file.path }}
        source={file.content.text}
      />
    )
  }

  if (isRepositoryHtmlPath(file.path)) {
    return (
      <RepositoryHtmlRenderer
        identity={`${file.path}\0${file.oid}`}
        key={`${file.path}:${file.oid}`}
        path={file.path}
        mode={htmlMode}
        source={file.content.text}
      />
    )
  }

  return (
    <pre className="min-h-full bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
      <code>{file.content.text}</code>
    </pre>
  )
}
