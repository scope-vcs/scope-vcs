import { cn } from '@/lib/utils'
import { ChevronDown } from 'lucide-react'
import { useEffect, useState, type ReactElement } from 'react'
import { useLandingView } from './landing-view'

/** A chevron at the foot of the hero that dissolves once the next section is on
 * screen and comes back at the top. */
export function ScrollCue({ target }: { target: string }): ReactElement {
  const view = useLandingView()
  const [gone, setGone] = useState(false)

  useEffect(() => {
    const next = document.getElementById(target)
    if (!next) return
    const update = () => setGone(next.getBoundingClientRect().top < innerHeight - 60)
    update()
    addEventListener('scroll', update, { passive: true })
    addEventListener('resize', update)
    return () => {
      removeEventListener('scroll', update)
      removeEventListener('resize', update)
    }
  }, [target])

  return (
    <a
      aria-label="Scroll to the next section"
      className={cn('landing-cue absolute bottom-6 left-1/2 grid size-11 -translate-x-1/2 place-items-center text-muted-foreground hover:text-foreground max-[901px]:hidden', gone && 'is-gone')}
      href={`#${target}`}
      tabIndex={view === 'public' && !gone ? undefined : -1}
    >
      <span className="relative size-[18px] overflow-hidden">
        <ChevronDown aria-hidden className="absolute left-0 top-0 size-[18px] stroke-[1.6]" />
        <ChevronDown aria-hidden className="absolute left-0 -top-[18px] size-[18px] stroke-[1.6]" />
      </span>
    </a>
  )
}
