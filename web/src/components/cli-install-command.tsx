import type { CliInstallCommands, CliPlatform } from '@/api/types'
import { CopyableCodeBlock } from '@/components/copyable-code-block'
import { cn } from '@/lib/utils'
import { useState, type ReactElement } from 'react'

const platformOptions = [
  { copyName: 'macOS and Linux', label: 'macOS / Linux', value: 'posix' },
  { copyName: 'Windows', label: 'Windows', value: 'windows' },
] as const

/**
 * Platform picker plus the matching install command. Owns the selected
 * platform unless the caller passes `platform` to share it between copies;
 * callers style the code block for their surface.
 */
export function CliInstallCommand({
  codeBlockClassName,
  commands,
  initialPlatform,
  onCopy,
  onPlatformChange,
  pickerClassName,
  platform: sharedPlatform,
}: {
  codeBlockClassName?: string
  commands: CliInstallCommands
  initialPlatform: CliPlatform
  onCopy?: () => void
  onPlatformChange?: (platform: CliPlatform) => void
  pickerClassName?: string
  platform?: CliPlatform
}): ReactElement {
  const [ownPlatform, setOwnPlatform] = useState<CliPlatform>(initialPlatform)
  const platform = sharedPlatform ?? ownPlatform
  const setPlatform = onPlatformChange ?? setOwnPlatform
  const option = platformOptions.find((item) => item.value === platform) ?? platformOptions[0]

  return (
    <div className="min-w-0">
      <fieldset className={cn('mb-3 flex min-w-0 gap-6 border-b border-border', pickerClassName)}>
        <legend className="sr-only">Operating system</legend>
        {platformOptions.map((item) => (
          <button
            aria-pressed={item.value === platform}
            className="-mb-px min-h-[35px] border-b-2 border-transparent bg-transparent pb-3 text-[13px] text-muted-foreground hover:text-foreground aria-pressed:border-success-strong aria-pressed:text-foreground pointer-coarse:min-h-11 max-[521px]:min-h-11"
            key={item.value}
            onClick={() => setPlatform(item.value)}
            type="button"
          >
            {item.label}
          </button>
        ))}
      </fieldset>
      <CopyableCodeBlock
        className={codeBlockClassName}
        copyLabel={`Copy ${option.copyName} install command`}
        key={platform}
        onCopy={onCopy}
        value={commands[platform]}
      />
    </div>
  )
}
