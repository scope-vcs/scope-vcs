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
const shell = 'mx-auto w-[calc(100%-64px)] max-w-[1120px] max-[521px]:w-[calc(100%-40px)]'
const sectionLayout = 'grid grid-cols-[minmax(0,.9fr)_minmax(0,1.1fr)] items-start gap-12 max-[1151px]:gap-8 max-[901px]:grid-cols-1'
const sectionHeading = 'text-[30px] leading-[1.15] font-medium tracking-[-.035em] max-[521px]:text-[27px]'
const sectionCopy = 'mt-4 max-w-[380px] text-[15px] leading-[1.7] text-landing-muted max-[901px]:max-w-[540px]'
const logo = 'block h-auto brightness-0 dark:invert'
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
      <header className={cn('topbar flex h-[76px] items-center justify-between max-[521px]:h-[72px]', shell)}>
        <div className="flex items-center gap-[17px] max-[521px]:gap-3">
          <Link aria-label="Scope home" to="/">
            <ScopeLogo className={cn(logo, 'w-[112px] max-[521px]:w-[100px]')} />
          </Link>
          <span className="border-l border-landing-line pl-[17px] font-mono text-xs leading-[normal] text-landing-muted max-[521px]:pl-3 max-[521px]:text-[10px] max-[361px]:hidden">pre-alpha</span>
        </div>
        <nav className="flex items-center gap-7 max-[521px]:gap-3 [&_button]:size-[38px] [&_button]:rounded-full [&_button]:bg-transparent [&_button:hover]:bg-landing-panel" aria-label="Account and appearance">
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
      <main className={cn(shell, 'outline-none [--landing-section-space:104px] max-[901px]:[--landing-section-space:80px]')} data-scope-page="landing" id="main-content" tabIndex={-1}>
        <section className={cn('pt-16 max-[901px]:gap-9 max-[901px]:pt-10 max-[521px]:pt-8', sectionLayout)} aria-labelledby="hero-title">
          <div className="min-w-0 max-[901px]:max-w-[640px]">
            <h1 id="hero-title" className="enter text-[clamp(42px,4vw,56px)] leading-[1.06] font-[500] tracking-[-.05em] max-[901px]:text-[48px] max-[521px]:text-[clamp(34px,8.7vw,42px)]">
              <span className="block">One repository.</span>{' '}
              <span className={cn(headlineLine, 'text-landing-muted')}>You choose</span>{' '}
              <span className={cn(headlineLine, 'text-landing-muted')}>what’s <span className="accent relative inline-block text-landing-green after:absolute after:right-0 after:-bottom-[5px] after:left-px after:h-0.5 after:origin-left after:bg-landing-green after:content-['']">public.</span></span>
            </h1>
            <p className="enter mt-5 max-w-[390px] text-base leading-[1.65] text-landing-muted [animation-delay:90ms] max-[901px]:max-w-[540px] max-[521px]:text-[15px]">
              Keep public and private code in one Git repository. Contributors clone
              only the files you share.
            </p>
            <div className="enter mt-6 flex flex-wrap items-center gap-x-5 gap-y-2 [animation-delay:180ms] max-[521px]:gap-x-4">
              <a href="#install" className="group inline-flex min-h-11 items-center justify-center gap-3 rounded-[5px] bg-landing-ink px-4 py-2.5 text-[13px] font-medium text-landing-bg transition-[transform,box-shadow] duration-180 hover:-translate-y-0.5 hover:shadow-[0_4px_0_var(--landing-line)] max-[521px]:gap-3.5 max-[521px]:px-3.5">
                Install Scope<ArrowDown className="icon transition-transform duration-200 group-hover:translate-y-0.5" aria-hidden />
              </a>
              <a href={sourceUrl} className="group inline-flex items-center gap-2 py-2.5 text-[14px] hover:text-landing-green max-[521px]:gap-[5px] max-[521px]:text-[13px]">
                Browse Scope’s source<ArrowUpRight className="icon w-[15px] transition-transform duration-200 group-hover:translate-x-0.5 group-hover:-translate-y-0.5" aria-hidden />
              </a>
            </div>
          </div>
          <RepositoryProjection />
        </section>
        <section className={cn('contributions pt-[var(--landing-section-space)] max-[901px]:gap-8', sectionLayout)} aria-labelledby="contribution-title">
          <div>
            <h2 id="contribution-title" className={sectionHeading}>Accept changes to<br />the code you share.</h2>
            <p className={sectionCopy}>
              Review changes from the public clone and merge them into your repository.
              Your private code stays private.
            </p>
          </div>
          <ContributionFlow />
        </section>
        <section className={cn('install scroll-mt-8 pt-[var(--landing-section-space)] pb-12 max-[901px]:gap-8', sectionLayout)} id="install" aria-labelledby="install-title">
          <div>
            <h2 id="install-title" className={sectionHeading}>Bring your repository.</h2>
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
