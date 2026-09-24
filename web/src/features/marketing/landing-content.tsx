import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CliInstallCommand } from '@/components/cli-install-command'
import { ScopeLogo } from '@/components/scope-logo'
import { ThemeToggle } from '@/components/theme-toggle'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ArrowUpRight } from 'lucide-react'
import type { CSSProperties, MouseEvent, ReactElement } from 'react'
import { heroCopy, installCommandAside, installTitle, mergeTitle, notes, sourceUrl, touchNavNote, type LandingView } from './landing-copy'
import { Heading, LandingViewProvider, Note, Swap } from './landing-view'
import { MergeGraph } from './merge-graph'
import { RepoPanel } from './repo-panel'
import { ScrollCue } from './scroll-cue'

const landingShell = 'mx-auto w-[calc(100%-64px)] max-w-[1120px] max-[521px]:w-[calc(100%-40px)]'
const column = 'relative min-w-0 scroll-mt-8'
const columnTitle = 'max-w-[12ch] text-[clamp(32px,3.6vw,48px)] leading-[1.04] font-medium tracking-[-.045em]'
const columnNote = 'absolute left-0 top-[calc(100%+24px)] max-w-[34ch]'
// The private copy can't scroll, so its longer command widens the box toward
// the window edge (the column starts 40px past the centre) and wraps beyond it.
// Its copy button would move with the wider box and can't be clicked, so it's
// hidden; the lens closes over the real one.
const privateCommand = 'w-max min-w-full max-w-[calc(50vw-56px)] [&_pre]:overflow-visible [&_pre]:whitespace-pre-wrap [&_button]:invisible max-[901px]:w-auto max-[901px]:max-w-none'
const rise = (delay: number) => ({ '--rise-delay': `${delay}ms` }) as CSSProperties

/** The whole page, once per view. Only the public view carries ids, headings
 * and accessible names; the private view is decoration under the lens. */
export function LandingContent({
  commands,
  initialPlatform,
  installCalled,
  onInstallCall,
  onPlatformChange,
  platform,
  view,
}: {
  commands: CliInstallCommands
  initialPlatform: CliPlatform
  installCalled: boolean
  onInstallCall: () => void
  onPlatformChange: (platform: CliPlatform) => void
  platform: CliPlatform
  view: LandingView
}): ReactElement {
  const isPublic = view === 'public'
  const shownCommands = isPublic ? commands : {
    posix: commands.posix + installCommandAside,
    windows: commands.windows + installCommandAside,
  }

  return (
    <LandingViewProvider view={view}>
      <div className={landingShell}>
        <header className="relative flex h-[76px] items-center justify-between">
          <Link aria-label="Scope home" to="/">
            <ScopeLogo className="landing-logo w-[112px] max-[521px]:w-[100px]" />
          </Link>
          <Note className="absolute left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 whitespace-nowrap max-[901px]:whitespace-normal max-[421px]:left-0 max-[421px]:top-[calc(100%-4px)] max-[421px]:translate-none" id="nav">
            <span className="pointer-coarse:hidden">{notes.nav}</span>
            <span className="hidden pointer-coarse:inline">{touchNavNote}</span>
          </Note>
          <nav aria-label={isPublic ? 'Account and appearance' : undefined} className="flex items-center gap-5 text-sm">
            <Link className="hover:text-success-strong" params={{ _splat: '' }} search={{ redirect_url: '/' }} to="/sign-in/$">Sign in</Link>
            <ThemeToggle />
          </nav>
        </header>

        <main className="outline-none" id={isPublic ? 'main-content' : undefined} tabIndex={isPublic ? -1 : undefined}>
          <section className="relative grid min-h-[min(calc(100dvh-76px),820px)] grid-cols-[minmax(0,1fr)_400px] items-center gap-24 pb-24 max-[901px]:min-h-0 max-[901px]:grid-cols-1 max-[901px]:gap-14 max-[901px]:pt-28 max-[421px]:pt-40">
            <Note className="absolute left-0 top-[7%] max-w-[40ch] max-[901px]:top-7 max-[421px]:top-16" id="heroTop" />
            <div className="relative min-w-0">
              <Heading className="text-[clamp(46px,6vw,84px)] leading-[.98] font-medium tracking-[-.058em]" level={1}>
                <span className="landing-rise block">{heroCopy.title}</span>{' '}
                <span className="landing-rise block text-muted-foreground" style={rise(80)}><Swap text={heroCopy.subtitle} /></span>
              </Heading>
              <p className="landing-rise mt-6 max-w-[36ch] text-[17px] leading-[1.6] text-muted-foreground" style={rise(180)}><Swap text={heroCopy.lede} /></p>
              <div className="landing-rise mt-8 flex flex-wrap items-center gap-x-6 gap-y-2" style={rise(260)}>
                <a className="inline-flex min-h-11 items-center rounded-md bg-foreground px-[18px] text-sm font-medium text-background transition-transform duration-200 hover:-translate-y-0.5" href="#install" onClick={(event) => callInstall(event, onInstallCall)}>Install Scope</a>
                <a className="inline-flex items-center gap-1.5 text-sm hover:text-success-strong" href={sourceUrl}>Source<ArrowUpRight aria-hidden className="size-4 stroke-[1.6]" /></a>
              </div>
              <Note className="absolute left-0 top-[calc(100%+44px)] max-[901px]:static max-[901px]:mt-5 max-[901px]:block" id="cta" />
            </div>
            <RepoPanel />
            <ScrollCue target="merge" />
          </section>

          {/* The second screen: two columns, headline above figure. */}
          <section className="relative grid grid-cols-2 items-start gap-x-20 gap-y-24 pt-8 pb-40 max-[901px]:grid-cols-1 max-[901px]:pt-10 max-[901px]:pb-40">
            <div className={column} id={isPublic ? 'merge' : undefined}>
              <Heading className={columnTitle} level={2}><Swap text={mergeTitle} /></Heading>
              <div className="mt-12"><MergeGraph /></div>
              <Note className={columnNote} id="merge" />
            </div>
            <div className={column} id={isPublic ? 'install' : undefined}>
              <Heading className={columnTitle} level={2}><Swap text={installTitle} /></Heading>
              <div className="mt-12 min-w-0" data-note="command">
                <CliInstallCommand
                  codeBlockClassName={cn('landing-terminal rounded-lg border-0 py-2 pl-4 pr-2 shadow-none [&_pre]:whitespace-pre [&_pre]:py-2.5 [&_pre]:pr-12 [&_pre]:text-sm [&_pre]:leading-6', installCalled && 'is-called', !isPublic && privateCommand)}
                  commands={shownCommands}
                  initialPlatform={initialPlatform}
                  onPlatformChange={onPlatformChange}
                  pickerClassName="platforms mb-4"
                  platform={platform}
                />
              </div>
              <Note className={columnNote} id="install" />
            </div>
            <Note className="absolute bottom-2 right-0 max-w-[24ch] text-right" id="corner" />
          </section>
        </main>

        <footer className="relative flex min-h-24 flex-wrap items-center justify-between gap-5 text-[13px] text-muted-foreground max-[901px]:pb-8">
          <Link aria-label="Scope home" to="/"><ScopeLogo className="landing-logo w-[84px] opacity-65" /></Link>
          <Note className="absolute left-1/2 top-1/2 max-w-[44ch] -translate-x-1/2 -translate-y-1/2 text-center max-[901px]:static max-[901px]:order-last max-[901px]:block max-[901px]:max-w-none max-[901px]:basis-full max-[901px]:translate-none max-[901px]:text-left" id="footer" />
          <nav aria-label={isPublic ? 'Project' : undefined} className="flex gap-6 [&_a:hover]:text-success-strong">
            <a href={sourceUrl}>Source</a>
            <Link to="/licenses">Licenses</Link>
          </nav>
        </footer>
      </div>
    </LandingViewProvider>
  )
}

/** Lights up the install command, since the command is what the button
 * offers. The page only scrolls when the command isn't already fully on
 * screen, so a visible command never moves. The URL stays put too: the router
 * scrolls to any hash it sees. */
function callInstall(event: MouseEvent<HTMLAnchorElement>, onInstallCall: () => void) {
  const install = document.getElementById('install')
  const command = install?.querySelector('[data-note="command"]')
  if (!install || !command) return
  event.preventDefault()
  const box = command.getBoundingClientRect()
  if (box.top < 0 || box.bottom > innerHeight) {
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches
    command.scrollIntoView({ behavior: reduced ? 'auto' : 'smooth', block: 'center' })
  }
  onInstallCall()
}
