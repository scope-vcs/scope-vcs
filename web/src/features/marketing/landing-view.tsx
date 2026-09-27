import { cn } from '@/lib/utils'
import { createContext, useContext, type ReactElement, type ReactNode } from 'react'
import { notes, type LandingView, type NoteId, type PairedText } from './landing-copy'

/** Which copy of the page a component is rendering in. The page renders once
 * per view; the private copy sits under the lens. */
const LandingViewContext = createContext<LandingView>('public')

export function LandingViewProvider({ children, view }: { children: ReactNode; view: LandingView }): ReactElement {
  return <LandingViewContext value={view}>{children}</LandingViewContext>
}

export function useLandingView(): LandingView {
  return useContext(LandingViewContext)
}

export function Swap({ text }: { text: PairedText }): ReactElement {
  return <span className="landing-swap">{text[useLandingView()]}</span>
}

/** Only the public copy carries real headings; the private copy is decorative. */
export function Heading({ children, className, level }: { children: ReactNode; className?: string; level: 1 | 2 }): ReactElement {
  const view = useLandingView()
  const Tag = view === 'public' ? (`h${level}` as const) : 'p'
  return <Tag className={className}>{children}</Tag>
}

/** A note the lens reveals. The public copy keeps it invisible but laid out, so
 * the finder can measure where it sits. */
export function Note({ children, className, id }: { children?: ReactNode; className?: string; id: NoteId }): ReactElement {
  return (
    <span aria-hidden className={cn('landing-note max-w-[34ch]', className)} data-note={id}>
      {children ?? notes[id]}
    </span>
  )
}
