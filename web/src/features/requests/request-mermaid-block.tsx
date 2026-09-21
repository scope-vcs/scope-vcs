import { useCallback, useMemo, useSyncExternalStore } from 'react'
import { useAuth } from '@clerk/tanstack-react-start'
import { useThemeType } from '@/lib/use-theme-type'
import { useRequestAttachments } from './request-attachment-context'
import {
  acquireRequestMermaid,
  requestMermaidIdentity,
  requestMermaidResource,
  retryRequestMermaid,
} from './request-mermaid-resource'
import { observeRequestMermaid } from './request-mermaid-visibility'

export function RequestMermaidBlock({ source }: { source: string }) {
  const { isLoaded, userId } = useAuth()
  const { viewerId } = useRequestAttachments()
  // Route data may still belong to the previous viewer while Clerk changes.
  // Unmount its observer/lease rather than rerendering discarded private data.
  if (isLoaded && (userId ?? 'anonymous') !== viewerId) return null
  return <RequestMermaidImage source={source} />
}

function RequestMermaidImage({ source }: { source: string }) {
  const { accessScope } = useRequestAttachments()
  const theme = useThemeType()
  const input = useMemo(() => ({ accessScope, source, theme }), [accessScope, source, theme])
  const identity = requestMermaidIdentity(input)
  const observe = useCallback((element: HTMLDivElement | null) => {
    if (!element) return
    let priority: 0 | 1 | null = null
    let release: (() => void) | undefined
    const unobserve = observeRequestMermaid(element, (next) => {
      if (priority === next) return
      release?.()
      priority = next
      release = next === null ? undefined : acquireRequestMermaid(input, next)
    })
    return () => { unobserve(); release?.() }
  }, [input])
  const subscribe = useCallback((notify: () => void) => requestMermaidResource.subscribe(identity, notify), [identity])
  const read = useCallback(() => requestMermaidResource.getSnapshot(identity), [identity])
  const snapshot = useSyncExternalStore(subscribe, read, requestMermaidResource.getServerSnapshot)

  // A theme refresh can keep its prior picture. Source/access changes cannot.
  const previous = requestMermaidResource.getSnapshot(requestMermaidIdentity({ ...input, theme: theme === 'dark' ? 'light' : 'dark' }))
  const result = snapshot.value ?? previous.value
  const showImage = useCallback((element: HTMLImageElement | null) => {
    if (!element || !result) return
    const url = URL.createObjectURL(new Blob([result.svg], { type: 'image/svg+xml' }))
    element.src = url
    return () => URL.revokeObjectURL(url)
  }, [result])
  const error = snapshot.error instanceof Error ? snapshot.error.message : snapshot.error ? 'This diagram could not be rendered.' : null

  return (
    <div className="my-3 min-w-0" data-mermaid-block ref={observe}>
      {result ? (
        <section aria-label="Diagram" className="max-w-full overflow-x-auto py-2">
          <img alt="Mermaid diagram. The diagram source is available below." className="max-w-none" height={result.height} ref={showImage} width={result.width} />
        </section>
      ) : error ? null : (
        <div aria-busy="true" className="min-h-32 border-l-2 border-border pl-3">
          <span className="sr-only">Diagram waiting to render</span>
        </div>
      )}
      {error ? (
        <output className="my-2 block text-sm text-muted-foreground">
          <span className="block">{error}</span>
          <button className="mt-1 underline underline-offset-4 hover:text-foreground" onClick={() => retryRequestMermaid(input, 0)} type="button">Retry diagram</button>
        </output>
      ) : null}
      <details className="text-sm text-muted-foreground" open={error ? true : undefined}>
        <summary className="w-fit cursor-pointer py-1 hover:text-foreground">View source</summary>
        <pre className="mt-2 max-h-80 overflow-auto bg-[var(--terminal-surface)] p-3 font-mono text-xs text-[var(--terminal-foreground)]"><code>{source}</code></pre>
      </details>
    </div>
  )
}
