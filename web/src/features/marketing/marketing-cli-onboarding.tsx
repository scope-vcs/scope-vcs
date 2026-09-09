import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { useRef, useState, type ReactElement } from 'react'

const platformOptions = [
  { copyName: 'macOS and Linux', label: 'macOS / Linux', value: 'posix' },
  { copyName: 'Windows', label: 'Windows', value: 'windows' },
] as const

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
  const [platform, setPlatform] = useState<CliPlatform>(initialPlatform)
  const nextStepsRef = useRef<HTMLDetailsElement>(null)
  const option = platformOptions.find((item) => item.value === platform) ?? platformOptions[0]

  return (
    <div className="min-w-0">
      <fieldset className="platforms mb-[18px] flex min-w-0 gap-6 border-b border-landing-line">
        <legend className="sr-only">Operating system</legend>
        {platformOptions.map((item) => (
          <button
            aria-pressed={item.value === platform}
            className="-mb-px min-h-[35px] border-b-2 border-transparent bg-transparent pb-3 text-[13px] text-landing-muted hover:text-landing-ink aria-pressed:border-landing-green aria-pressed:text-landing-ink pointer-coarse:min-h-11 max-[521px]:min-h-11"
            key={item.value}
            onClick={() => setPlatform(item.value)}
            type="button"
          >
            {item.label}
          </button>
        ))}
      </fieldset>
      <CopyableCodeBlock
        className="terminal flex items-start gap-3 rounded-[5px] border border-landing-line bg-landing-panel px-3.5 py-[17px] text-landing-ink shadow-none before:font-mono before:text-xs before:leading-[1.8] before:text-landing-green before:content-['›'] max-[521px]:gap-2 max-[521px]:px-2.5 max-[521px]:py-3.5 [&_pre]:m-0 [&_pre]:min-w-0 [&_pre]:flex-1 [&_pre]:p-0 [&_pre]:pr-12 [&_pre]:text-xs [&_pre]:leading-[1.8] max-[521px]:[&_pre]:text-[11px] [&_button]:h-9 [&_button]:w-10 [&_button]:rounded-[3px] [&_button]:border [&_button]:border-landing-line [&_button]:bg-landing-paper [&_button]:text-landing-muted [&_button:hover]:border-landing-green [&_button:hover]:text-landing-green"
        copyLabel={`Copy ${option.copyName} install command`}
        key={platform}
        onCopy={() => {
          const details = nextStepsRef.current
          if (details && !details.open) {
            details.open = true
            details.querySelector('summary')?.focus()
          }
        }}
        value={commands[platform]}
      />
      <details className="group mt-6" ref={nextStepsRef}>
        <summary className="flex w-fit cursor-pointer list-none items-center gap-2 py-2 text-[13px] text-landing-muted before:w-3 before:font-mono before:text-base before:leading-[normal] before:content-['+'] hover:text-landing-ink group-open:before:content-['−'] [&::-webkit-details-marker]:hidden">Already installed?</summary>
        <ol className="mt-3.5 grid gap-[15px]">
          {nextSteps.map((step, index) => (
            <li className="grid grid-cols-[24px_minmax(0,1fr)] gap-2" key={step.command}>
              <span className="pt-[3px] font-mono text-xs leading-[normal] text-landing-faint" aria-hidden>{index + 1}</span>
              <div>
                <p className="mb-1 text-[13px] text-landing-muted">{step.description}</p>
                <code className="font-mono text-[13px] leading-[normal]">{step.command}</code>
              </div>
            </li>
          ))}
        </ol>
      </details>
    </div>
  )
}
