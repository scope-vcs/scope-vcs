import type { CliInstallState } from '@/api/types'
import { CliInstallCommand } from '@/components/cli-install-command'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { Link } from '@tanstack/react-router'
import type { ReactNode } from 'react'

/**
 * The owner's empty state: how to get a first repository onto Scope from the
 * terminal. Visitors see the plain empty state in `RepoList` instead.
 */
export function FirstRepositoryWalkthrough({
  cliInstallCommands,
  initialCliPlatform,
}: CliInstallState) {
  return (
    <section aria-labelledby="first-repository-title" className="mt-8 max-w-[640px]">
      <h2 className="text-[15px] font-semibold leading-5" id="first-repository-title">
        No repositories yet
      </h2>
      <p className="mt-1 max-w-[52ch] text-sm leading-5 text-muted-foreground">
        Repositories are created from the terminal. Three steps, then this page
        fills in on its own.
      </p>

      <ol className="mt-5 divide-y divide-border border-t border-border">
        <WalkthroughStep number={1} title="Install the CLI">
          <CliInstallCommand
            commands={cliInstallCommands}
            initialPlatform={initialCliPlatform}
          />
        </WalkthroughStep>
        <WalkthroughStep
          description="Run this inside any Git repository, committed or not. It opens your browser to sign in, creates the repository on Scope, and adds the remote."
          number={2}
          title="Initialize a Git repository"
        >
          <CopyableCodeBlock copyLabel="Copy init command" value="scope init" />
        </WalkthroughStep>
        <WalkthroughStep
          description="Commit the generated Scope files, then publish. The repository appears here as soon as the push lands."
          number={3}
          title="Push your first version"
        >
          <CopyableCodeBlock copyLabel="Copy push command" value="scope push --main" />
        </WalkthroughStep>
      </ol>

      <p className="mt-4 flex flex-wrap gap-x-5 gap-y-1.5 text-xs leading-4 text-muted-foreground">
        <span>Already have the CLI? Start at step 2.</span>
        <Link className="underline underline-offset-2 hover:text-foreground" to="/account">
          Manage CLI sessions
        </Link>
      </p>
    </section>
  )
}

function WalkthroughStep({
  children,
  description,
  number,
  title,
}: {
  children: ReactNode
  description?: string
  number: number
  title: string
}) {
  return (
    <li className="grid grid-cols-[28px_minmax(0,1fr)] gap-x-3 py-5">
      <span aria-hidden className="pt-0.5 font-mono text-xs leading-5 text-muted-foreground">
        {number}
      </span>
      <div className="min-w-0">
        <h3 className="text-sm font-semibold leading-5">{title}</h3>
        {description && (
          <p className="mt-1 max-w-[56ch] text-sm leading-5 text-muted-foreground">
            {description}
          </p>
        )}
        <div className="mt-3">{children}</div>
      </div>
    </li>
  )
}
