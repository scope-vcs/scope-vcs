import { MAIN_CONTENT_ID } from './main-content'
import type { ReactNode } from 'react'

export function AppShell({
  children,
  header,
}: {
  children: ReactNode
  header?: () => ReactNode
}) {
  return (
    <div className="flex h-dvh min-h-0 flex-col overflow-hidden bg-background text-foreground">
      <a
        className="fixed left-4 top-3 z-50 -translate-y-16 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background shadow-md focus:translate-y-0"
        href={`#${MAIN_CONTENT_ID}`}
      >
        Skip to content
      </a>
      {header?.()}
      <main
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        data-scroll-restoration-id={MAIN_CONTENT_ID}
        id={MAIN_CONTENT_ID}
        tabIndex={-1}
      >
        {children}
      </main>
    </div>
  )
}
