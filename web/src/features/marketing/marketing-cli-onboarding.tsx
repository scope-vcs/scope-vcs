import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CliInstallCommand } from '@/components/cli-install-command'
import { useRef, type ReactElement } from 'react'

const nextSteps = [
  { command: 'scope login', description: 'Sign in from your terminal.' },
  { command: 'scope init', description: 'Run inside an existing Git repository.' },
  { command: 'scope push --main', description: 'Review and push your first version.' },
] as const

export function MarketingCliOnboarding({
  commands,
  initialPlatform,
}: {
  commands: CliInstallCommands
  initialPlatform: CliPlatform
}): ReactElement {
  const nextStepsRef = useRef<HTMLDetailsElement>(null)

  return (
    <div className="min-w-0">
      <CliInstallCommand
        codeBlockClassName="terminal flex items-start gap-3 rounded-[5px] border border-border bg-muted px-3.5 py-[17px] text-foreground shadow-none before:font-mono before:text-xs before:leading-[1.8] before:text-success-strong before:content-['›'] max-[521px]:gap-2 max-[521px]:px-2.5 max-[521px]:py-3.5 [&_pre]:m-0 [&_pre]:min-w-0 [&_pre]:flex-1 [&_pre]:p-0 [&_pre]:pr-12 [&_pre]:text-xs [&_pre]:leading-[1.8] max-[521px]:[&_pre]:text-[11px] [&_button]:h-9 [&_button]:w-10 [&_button]:rounded-[3px] [&_button]:border [&_button]:border-border [&_button]:bg-card [&_button]:text-muted-foreground [&_button:hover]:border-success-strong [&_button:hover]:text-success-strong"
        commands={commands}
        initialPlatform={initialPlatform}
        onCopy={() => {
          const details = nextStepsRef.current
          if (details && !details.open) {
            details.open = true
            details.querySelector('summary')?.focus()
          }
        }}
        pickerClassName="platforms mb-[18px]"
      />
      <details className="group mt-6" ref={nextStepsRef}>
        <summary className="flex w-fit cursor-pointer list-none items-center gap-2 py-2 text-[13px] text-muted-foreground before:w-3 before:font-mono before:text-base before:leading-[normal] before:content-['+'] hover:text-foreground group-open:before:content-['−'] [&::-webkit-details-marker]:hidden">Already installed?</summary>
        <ol className="mt-3.5 grid gap-[15px]">
          {nextSteps.map((step, index) => (
            <li className="grid grid-cols-[24px_minmax(0,1fr)] gap-2" key={step.command}>
              <span className="pt-[3px] font-mono text-xs leading-[normal] text-muted-foreground/70" aria-hidden>{index + 1}</span>
              <div>
                <p className="mb-1 text-[13px] text-muted-foreground">{step.description}</p>
                <code className="font-mono text-[13px] leading-[normal]">{step.command}</code>
              </div>
            </li>
          ))}
        </ol>
      </details>
    </div>
  )
}
