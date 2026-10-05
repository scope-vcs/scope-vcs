import { cn } from '@/lib/utils'
import { createContext, useContext, type ReactElement, type ReactNode } from 'react'
import { notes, type LandingView, type NoteId, type PairedText } from './landing-copy'

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

export function Heading({ children, className, level }: { children: ReactNode; className?: string; level: 1 | 2 }): ReactElement {
  const view = useLandingView()
  const Tag = view === 'public' ? (`h${level}` as const) : 'p'
  return <Tag className={className}>{children}</Tag>
}

export function Note({ children, className, id }: { children?: ReactNode; className?: string; id: NoteId }): ReactElement {
  return (
    <span aria-hidden className={cn('landing-note max-w-[34ch]', className)} data-note={id}>
      {children ?? notes[id]}
    </span>
  )
}
