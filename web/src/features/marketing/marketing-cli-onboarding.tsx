import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { useRef, useState, type ReactElement } from 'react'
import './marketing-cli-onboarding.css'

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
    <div className="install-ui">
      <fieldset className="platforms">
        <legend className="sr-only">Operating system</legend>
        {platformOptions.map((item) => (
          <button
            aria-pressed={item.value === platform}
            className="platform"
            key={item.value}
            onClick={() => setPlatform(item.value)}
            type="button"
          >
            {item.label}
          </button>
        ))}
      </fieldset>
      <CopyableCodeBlock
        className="terminal"
        copyLabel={`Copy ${option.copyName} install command`}
        key={platform}
        onCopy={() => {
          if (nextStepsRef.current) nextStepsRef.current.open = true
        }}
        value={commands[platform]}
      />
      <details ref={nextStepsRef}>
        <summary>Already installed?</summary>
        <ol className="steps">
          {nextSteps.map((step, index) => (
            <li key={step.command}>
              <span className="step-number" aria-hidden>{index + 1}</span>
              <div>
                <p>{step.description}</p>
                <code>{step.command}</code>
              </div>
            </li>
          ))}
        </ol>
      </details>
    </div>
  )
}
