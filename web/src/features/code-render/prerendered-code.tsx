import DOMPurify from 'dompurify'
import { useLayoutEffect, useRef } from 'react'

export function mountPrerenderedCode(container: HTMLElement, html: string) {
  const fragment = DOMPurify.sanitize(html, {
    RETURN_DOM_FRAGMENT: true,
  })
  const host = document.createElement('diffs-container')
  const root = host.attachShadow({ mode: 'open' })
  root.replaceChildren(fragment)
  container.replaceChildren(host)
  return { host, root }
}

export function CodeSourceView({
  highlightedHtml,
  source,
}: {
  highlightedHtml: string | null
  source: string
}) {
  return highlightedHtml === null ? (
    <pre className="min-h-full overflow-x-auto bg-background p-5 font-mono text-xs leading-5 whitespace-pre text-foreground sm:p-7">
      <code>{source}</code>
    </pre>
  ) : (
    <PrerenderedCode html={highlightedHtml} />
  )
}

function PrerenderedCode({ html }: { html: string }) {
  const containerRef = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    if (containerRef.current) mountPrerenderedCode(containerRef.current, html)
  }, [html])

  return (
    <div
      className="prerendered-code min-h-full"
      ref={containerRef}
      suppressHydrationWarning
    />
  )
}
