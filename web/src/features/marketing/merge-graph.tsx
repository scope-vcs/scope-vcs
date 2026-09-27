import { cn } from '@/lib/utils'
import { useEffect, useRef, useState, type CSSProperties, type ReactElement } from 'react'
import { useLandingView } from './landing-view'

/** The logo's two lanes. The public copy shows only the public lane; the lens
 * reveals `main` and the merge back into it. Draws itself once scrolled to. */
export function MergeGraph(): ReactElement {
  const view = useLandingView()
  const ref = useRef<SVGSVGElement>(null)
  const [drawn, setDrawn] = useState(false)

  useEffect(() => {
    const graph = ref.current
    if (!graph) return
    const observer = new IntersectionObserver(([entry]) => {
      if (!entry?.isIntersecting) return
      setDrawn(true)
      observer.disconnect()
    }, { threshold: .45 })
    observer.observe(graph)
    return () => observer.disconnect()
  }, [])

  return (
    <svg
      aria-label={view === 'public' ? 'Contributors commit to the public clone; you merge their changes into your repository.' : undefined}
      className={cn('merge-graph h-auto w-full overflow-visible', drawn && 'is-drawn')}
      ref={ref}
      role={view === 'public' ? 'img' : undefined}
      viewBox="0 40 600 100"
    >
      <g className="landing-private-only is-kept">
        <path className="lane-main" d="M28 90C64 90 60 50 100 50H540" pathLength={1} />
        <path className="lane-public" d="M28 90C64 90 60 130 100 130" pathLength={1} />
        <path className="lane-public" d="M420 130C462 130 454 50 496 50" pathLength={1} />
        <circle className="dot-main" cx="28" cy="90" r="10" style={{ '--pop-delay': '0ms' } as CSSProperties} />
        <circle className="dot-main" cx="190" cy="50" r="6" style={{ '--pop-delay': '500ms' } as CSSProperties} />
        <circle className="dot-main" cx="320" cy="50" r="6" style={{ '--pop-delay': '700ms' } as CSSProperties} />
        <circle className="ring-main" cx="496" cy="50" r="9" style={{ '--pop-delay': '1100ms' } as CSSProperties} />
        <text x="556" y="54">main</text>
      </g>
      <path className="lane-public" d="M100 130H540" pathLength={1} />
      <circle className="ring-public" cx="100" cy="130" r="9" style={{ '--pop-delay': '200ms' } as CSSProperties} />
      <circle className="dot-public" cx="190" cy="130" r="6" style={{ '--pop-delay': '500ms' } as CSSProperties} />
      <circle className="dot-public" cx="290" cy="130" r="6" style={{ '--pop-delay': '700ms' } as CSSProperties} />
      <circle className="ring-public" cx="420" cy="130" r="9" style={{ '--pop-delay': '900ms' } as CSSProperties} />
      <text x="556" y="134">public</text>
    </svg>
  )
}
