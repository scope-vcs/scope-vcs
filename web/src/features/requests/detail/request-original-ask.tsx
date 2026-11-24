import { cn } from '@/lib/utils'
import { useId, useLayoutEffect, useRef, useState } from 'react'
import { RequestDiscussionMarkdown } from '../discussion/request-discussion-markdown'

const FOLDED_MAX_HEIGHT_PX = 168

export function RequestOriginalAsk({ login, text }: { login: string; text: string }) {
  const body = useRef<HTMLDivElement>(null)
  const bodyId = useId()
  const [overflows, setOverflows] = useState(false)
  const [open, setOpen] = useState(false)

  useLayoutEffect(() => {
    const element = body.current
    if (!element) return
    const measure = () => setOverflows(element.scrollHeight > FOLDED_MAX_HEIGHT_PX)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(element)
    return () => observer.disconnect()
  }, [text])

  const folded = overflows && !open

  return (
    <section className="mt-6 border-l-2 border-border-strong pl-4">
      <h2 className="text-sm leading-6">
        <span className="font-semibold">Original request</span>{' '}
        <span className="text-muted-foreground">by {login}</span>
      </h2>
      <div
        className={cn('relative overflow-hidden', folded && 'request-original-ask-folded')}
        id={bodyId}
        ref={body}
        style={folded ? { maxHeight: FOLDED_MAX_HEIGHT_PX } : undefined}
      >
        <RequestDiscussionMarkdown source={text} />
      </div>
      {overflows ? (
        <button
          aria-controls={bodyId}
          aria-expanded={open}
          className="mt-1 rounded-sm text-xs text-muted-foreground hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          onClick={() => setOpen((current) => !current)}
          type="button"
        >
          {open ? 'Show less' : 'Show all'}
        </button>
      ) : null}
    </section>
  )
}
