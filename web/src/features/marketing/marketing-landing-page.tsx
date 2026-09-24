import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { cn } from '@/lib/utils'
import { useThemeType } from '@/lib/use-theme-type'
import { createRef, useCallback, useEffect, useRef, useState, type ReactElement } from 'react'
import { LandingContent } from './landing-content'
import { LensCursor } from './lens/lens-cursor'
import { LensRing } from './lens/lens-ring'
import { useLens, type LensElements } from './lens/use-lens'
import { fireConfetti } from './notes/confetti'
import { FoundTally } from './notes/found-tally'
import { useNoteFinder } from './notes/use-note-finder'
import './marketing-landing-page.css'

/**
 * The signed-out landing page. It renders its content twice: the public copy
 * people read and click, and a private copy in the opposite theme that only
 * shows through the lens following the cursor.
 */
export function MarketingLandingPage({
  cliInstallCommands,
  initialCliPlatform,
}: {
  cliInstallCommands: CliInstallCommands
  initialCliPlatform: CliPlatform
}): ReactElement {
  const inverseTheme = useThemeType() === 'dark' ? 'light' : 'dark'
  const [platform, setPlatform] = useState(initialCliPlatform)
  const [elements] = useState<LensElements>(() => ({
    page: createRef(), privateLayer: createRef(), ring: createRef(), edge: createRef(), grip: createRef(), minorTicks: createRef(),
    majorTicks: createRef(), ticks: createRef(), label: createRef(), cursor: createRef(), tally: createRef(),
  }))
  const lens = useLens(elements)
  const stopConfetti = useRef<() => void>(undefined)
  const celebrate = useCallback(() => { stopConfetti.current = fireConfetti() }, [])
  useEffect(() => () => stopConfetti.current?.(), [])
  const progress = useNoteFinder(elements.page, lens.frame, celebrate)
  const content = { commands: cliInstallCommands, initialPlatform: initialCliPlatform, onPlatformChange: setPlatform, platform }

  return (
    <div className={cn('marketing-page landing relative min-h-dvh overflow-clip bg-background font-sans text-base leading-normal text-foreground antialiased', lens.ready && 'lens-ready', lens.on ? 'lens-on' : 'lens-off', lens.holding && 'lens-holding')} ref={elements.page}>
      <a className="fixed top-2.5 left-2.5 z-40 -translate-y-[160%] bg-foreground px-4 py-2.5 text-background focus:translate-y-0" href="#main-content">Skip to content</a>
      <div className="landing-layer" data-view="public">
        <LandingContent {...content} view="public" />
      </div>
      <div aria-hidden className={cn('landing-layer landing-private', inverseTheme)} data-view="private" inert ref={elements.privateLayer}>
        <LandingContent {...content} view="private" />
      </div>
      <LensRing
        elements={elements}
        highlight={progress.justFound}
        inverseTheme={inverseTheme}
        label={progress.justFound ? progress.found === progress.total ? 'found them all' : `found ${progress.found} of ${progress.total}` : lens.touch ? 'drag' : 'hold'}
        showLabel={progress.justFound || lens.hint}
      />
      <LensCursor ref={elements.cursor} />
      <FoundTally found={progress.found} inverseTheme={inverseTheme} moreBelow={progress.moreBelow} ref={elements.tally} total={progress.total} visible={lens.holding} />
      <p className={cn('landing-toast pointer-events-none fixed bottom-5 left-6 z-[21] font-mono text-xs text-muted-foreground', lens.message && 'is-visible')} role="status">{lens.message}</p>
    </div>
  )
}
