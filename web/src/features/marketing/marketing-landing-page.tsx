import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { ScopeLogo } from '@/components/scope-logo'
import { ThemeToggle } from '@/components/theme-toggle'
import { cn } from '@/lib/utils'
import { Link } from '@tanstack/react-router'
import { ArrowDown, ArrowUpRight } from 'lucide-react'
import type { ReactElement } from 'react'
import { ContributionFlow } from './contribution-flow'
import { MarketingCliOnboarding } from './marketing-cli-onboarding'
import { RepositoryProjection } from './repository-projection'
import './marketing-landing-page.css'
import './repository-projection.css'
import './contribution-flow.css'

const sourceUrl = 'https://scopevcs.com/adamblumoff/scope-vcs'
const shell = 'mx-auto w-[calc(100%-96px)] max-w-[1280px] min-[1500px]:max-w-[1320px] max-[1151px]:w-[calc(100%-64px)] max-[521px]:w-[calc(100%-40px)]'
const sectionLayout = 'grid grid-cols-[minmax(0,.9fr)_minmax(0,1.1fr)] gap-[60px] min-[1500px]:gap-x-20 max-[1151px]:grid-cols-[minmax(0,.85fr)_minmax(0,1.15fr)] max-[1151px]:gap-[35px] max-[901px]:grid-cols-1'
const sectionHeading = 'text-[clamp(28px,2.8vw,39px)] leading-[1.14] font-medium tracking-[-.035em]'
const sectionCopy = 'section-copy mt-[18px] max-w-[400px] text-base leading-[1.7] text-landing-muted max-[901px]:max-w-[540px]'
const logo = 'logo block h-auto brightness-0 dark:invert'
const headlineLine = 'block max-[901px]:inline'

export function MarketingLandingPage({
  cliInstallCommands,
  initialCliPlatform,
}: {
  cliInstallCommands: CliInstallCommands
  initialCliPlatform: CliPlatform
}): ReactElement {
  return (
    <div className="marketing-page min-h-dvh bg-landing-bg font-sans text-base leading-normal text-landing-ink antialiased scheme-light dark:scheme-dark">
      <a className="fixed top-2.5 left-2.5 z-10 -translate-y-[160%] bg-landing-ink px-4 py-2.5 text-landing-bg focus:translate-y-0" href="#main-content">Skip to content</a>
      <header className={cn('topbar flex h-[88px] items-center justify-between max-[521px]:h-[72px]', shell)}>
        <div className="flex items-center gap-[17px] max-[521px]:gap-3">
          <Link aria-label="Scope home" to="/">
            <ScopeLogo className={cn(logo, 'w-[126px] max-[521px]:w-[108px]')} />
          </Link>
          <span className="border-l border-landing-line pl-[17px] font-mono text-xs leading-[normal] text-landing-muted max-[521px]:pl-3 max-[521px]:text-[10px] max-[361px]:hidden">pre-alpha</span>
        </div>
        <nav className="top-actions flex items-center gap-7 max-[521px]:gap-3 [&_button]:size-[38px] [&_button]:rounded-full [&_button]:bg-transparent [&_button:hover]:bg-landing-panel" aria-label="Account and appearance">
          <Link
            className="text-[14px] hover:text-landing-green max-[521px]:text-[13px]"
            params={{ _splat: '' }}
            search={{ redirect_url: '/' }}
            to="/sign-in/$"
          >
            Sign in
          </Link>
          <ThemeToggle />
        </nav>
      </header>
      <main className={cn(shell, 'outline-none [--landing-section-space:clamp(96px,11vw,160px)]')} id="main-content" tabIndex={-1}>
        <section className={cn('hero items-center pt-[98px] min-[1500px]:pt-[115px] max-[1151px]:pt-[75px] max-[901px]:gap-[55px] max-[901px]:pt-16 max-[521px]:gap-[42px] max-[521px]:pt-[43px]', sectionLayout)} aria-labelledby="hero-title">
          <div className="min-w-0 max-[901px]:max-w-[640px]">
            <h1 id="hero-title" className="enter text-[clamp(46px,4.7vw,68px)] leading-[1.045] font-[540] tracking-[-.055em] min-[1500px]:text-7xl max-[1151px]:text-[49px] max-[901px]:text-[clamp(48px,7.5vw,65px)] max-[521px]:text-[clamp(39px,10.8vw,54px)] max-[521px]:leading-[1.06] max-[521px]:tracking-[-.05em]">
              <span className="block">One repository.</span>{' '}
              <span className={cn(headlineLine, 'text-landing-muted')}>You choose</span>{' '}
              <span className={cn(headlineLine, 'text-landing-muted')}>what’s <span className="accent relative inline-block text-landing-green after:absolute after:right-0 after:-bottom-[5px] after:left-px after:h-0.5 after:origin-left after:bg-landing-green after:content-['']">public.</span></span>
            </h1>
            <p className="intro enter mt-[27px] max-w-[425px] text-lg leading-[1.65] text-landing-muted [animation-delay:90ms] max-[1151px]:text-[17px] max-[901px]:max-w-[540px] max-[521px]:mt-6 max-[521px]:text-base">
              Keep public and private code in one Git repository. Contributors clone
              only the files you share.
            </p>
            <div className="hero-actions enter mt-[30px] flex flex-wrap items-center gap-5 [animation-delay:180ms] max-[1151px]:gap-x-[18px] max-[1151px]:gap-y-2.5 max-[901px]:gap-[22px] max-[521px]:mt-[25px] max-[521px]:gap-x-[17px] max-[521px]:gap-y-2 max-[361px]:gap-x-[15px] max-[361px]:gap-y-1.5">
              <a href="#install" className="group inline-flex min-h-[46px] items-center justify-center gap-5 rounded-[5px] bg-landing-ink px-[18px] py-3 text-[14px] font-[550] text-landing-bg transition-[transform,box-shadow] duration-180 hover:-translate-y-0.5 hover:shadow-[0_4px_0_var(--landing-line)] max-[521px]:min-h-11 max-[521px]:gap-3.5 max-[521px]:px-3.5 max-[521px]:text-[13px]">
                Install Scope<ArrowDown className="icon transition-transform duration-200 group-hover:translate-y-0.5" aria-hidden />
              </a>
              <a href={sourceUrl} className="group inline-flex items-center gap-2 py-2.5 text-[14px] hover:text-landing-green max-[521px]:gap-[5px] max-[521px]:text-[13px]">
                Browse Scope’s source<ArrowUpRight className="icon w-[15px] transition-transform duration-200 group-hover:translate-x-0.5 group-hover:-translate-y-0.5" aria-hidden />
              </a>
            </div>
          </div>
          <RepositoryProjection />
        </section>
        <section className={cn('contributions items-center pt-[var(--landing-section-space)] max-[901px]:gap-8', sectionLayout)} aria-labelledby="contribution-title">
          <div>
            <h2 id="contribution-title" className={sectionHeading}>Accept changes to<br />the code you share.</h2>
            <p className={sectionCopy}>
              Review changes from the public clone and merge them into your repository.
              Your private code stays private.
            </p>
          </div>
          <ContributionFlow />
        </section>
        <section className={cn('install scroll-mt-8 items-start pt-[var(--landing-section-space)] pb-[75px] max-[901px]:gap-8', sectionLayout)} id="install" aria-labelledby="install-title">
          <div>
            <h2 id="install-title" className="text-[32px] leading-[1.14] font-medium tracking-[-.035em]">Bring your repository.</h2>
            <p className={sectionCopy}>Install the CLI, then connect an existing Git repository.</p>
          </div>
          <MarketingCliOnboarding commands={cliInstallCommands} initialPlatform={initialCliPlatform} />
        </section>
      </main>
      <footer className={cn('footer flex min-h-[90px] items-center justify-between gap-5 text-xs leading-normal text-landing-muted max-[521px]:flex-col max-[521px]:items-start max-[521px]:justify-center max-[521px]:gap-[17px] max-[521px]:py-[25px] [&_a:hover]:text-landing-green', shell)}>
        <Link aria-label="Scope home" to="/"><ScopeLogo className={cn(logo, 'w-[84px] opacity-65')} /></Link>
        <nav className="flex gap-[23px] max-[521px]:gap-5" aria-label="Project">
          <a href={sourceUrl}>Source code ↗</a>
          <Link to="/licenses">Licenses</Link>
        </nav>
      </footer>
    </div>
  )
}
