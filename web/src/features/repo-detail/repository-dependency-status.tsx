import { Popover } from '@/components/ui/popover'
import { DependencyCheckDetailsBody, type DependencyCheckViewProps } from './repository-dependency-check-view'

export function RepositoryDependencyStatus({ onSelectFilePath, presentation, views }: DependencyCheckViewProps) {
  if (presentation.kind !== 'status') return null
  const label = (
    <>
      <span aria-hidden="true" className="size-1.5 shrink-0 rounded-full bg-current" />
      {presentation.label}
    </>
  )
  const { details } = presentation
  if (!details) {
    return <output className="flex items-center gap-2 px-2 text-xs text-warning-strong">{label}</output>
  }
  return (
    <Popover
      align="auto"
      className="w-[min(32rem,calc(100vw-2rem))] text-xs"
      label="Repository dependency check"
      panel={(close) => (
        <DependencyCheckDetailsBody
          details={details}
          onSelectFilePath={(path) => {
            onSelectFilePath(path)
            close()
          }}
          views={views}
        />
      )}
      trigger={(props) => (
        <button
          className="flex h-8 cursor-pointer items-center gap-2 rounded px-2 text-xs text-warning-strong hover:bg-warning-soft focus-visible:outline-2 focus-visible:outline-ring aria-expanded:bg-warning-soft"
          type="button"
          {...props}
        >
          {label}
        </button>
      )}
    />
  )
}
