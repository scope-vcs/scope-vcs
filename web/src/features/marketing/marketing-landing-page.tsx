import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { PageRail } from '@/components/page-header'
import { ScopeLogo } from '@/components/scope-logo'
import { Button } from '@/components/ui/button'
import { Link } from '@tanstack/react-router'
import type { ReactElement } from 'react'
import { MarketingCliOnboarding } from './marketing-cli-onboarding'
import { RepositoryProjection } from './repository-projection'

const authParams = { _splat: '' }
const authRedirect = { redirect_url: '/' }
export function MarketingLandingPage({
  cliInstallCommands,
  initialCliPlatform,
}: {
  cliInstallCommands: CliInstallCommands
  initialCliPlatform: CliPlatform
}): ReactElement {
  return (
    <div className="marketing-page min-h-dvh text-foreground">
      <a
        className="fixed left-4 top-3 z-50 -translate-y-16 rounded-md bg-foreground px-3 py-2 text-sm font-medium text-background shadow-md focus:translate-y-0"
        href="#main-content"
      >
        Skip to content
      </a>

      <div className="grid min-h-dvh grid-rows-[66px_1fr] sm:grid-rows-[74px_1fr]">
        <MarketingHeader />

        <PageRail
          as="main"
          className="marketing-arena py-10 outline-none sm:py-14"
          id="main-content"
          tabIndex={-1}
        >
          <div className="marketing-hero">
            <section className="marketing-copy">
              <h1 className="text-[clamp(2rem,7.8vw,4.6rem)] font-semibold leading-[1.05] tracking-[-0.06em]">
                <span className="block">One repository.</span>
                <span className="block text-muted-foreground">Public and private.</span>
              </h1>
              <p className="mt-6 max-w-[540px] text-lg leading-relaxed text-muted-foreground">
                Keep public and private code in one Git repository. The public clones
                only what you choose to share.
              </p>
              <p className="mt-4 text-sm leading-relaxed text-muted-foreground">
                One codebase. No split repositories. No synchronization scripts.
              </p>
            </section>
            <RepositoryProjection />
            <MarketingCliOnboarding
              commands={cliInstallCommands}
              initialPlatform={initialCliPlatform}
            />
          </div>

          <section className="marketing-explanation" aria-labelledby="request-workflow-title">
            <h2 className="text-2xl font-semibold tracking-tight" id="request-workflow-title">
              prepare privately, submit when ready
            </h2>
            <p className="mt-4 max-w-[700px] text-base leading-relaxed text-muted-foreground">
              Draft requests stay private to their participants. Submit when the work is
              ready for the maintainer queue, then keep the discussion and changes
              together as you revise.
            </p>
            <ol className="mt-6 flex flex-wrap items-center gap-x-4 gap-y-2 text-sm">
              <li>private draft</li>
              <li><span aria-hidden className="mr-4 text-muted-foreground">→</span>submit request</li>
              <li className="font-medium"><span aria-hidden className="mr-4 text-muted-foreground">→</span>maintainer queue</li>
            </ol>
          </section>
        </PageRail>
      </div>
    </div>
  )
}

function MarketingHeader(): ReactElement {
  return (
    <PageRail as="header" className="flex h-full items-center justify-between border-b border-border/80">
      <Link
        aria-label="Scope home"
        className="group flex items-center rounded-md focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring"
        to="/"
      >
        <ScopeLogo className="w-[118px] transition-transform duration-150 group-hover:-translate-y-px sm:w-[126px] motion-reduce:transform-none" />
      </Link>

      <nav aria-label="Account">
        <Button asChild className="h-10 px-3 sm:px-4" variant="ghost">
          <Link params={authParams} search={authRedirect} to="/sign-in/$">
            Sign in
          </Link>
        </Button>
      </nav>
    </PageRail>
  )
}
