import { ChevronRight } from 'lucide-react'
import type { DependencyCheckPresentation } from './repository-dependency-model'

export function RepositoryDependencyCheckView({
  onSelectFilePath,
  presentation,
}: {
  onSelectFilePath: (path: string) => void
  presentation: DependencyCheckPresentation
}) {
  if (presentation.kind === 'plain') {
    return (
      <div aria-label="Repository dependency check" className="flex min-h-11 flex-wrap items-center gap-x-3 gap-y-1 border-b border-border px-5 py-2.5 text-xs sm:px-6 lg:px-8">
        <span className={presentation.tone === 'warning' ? 'text-warning-strong' : 'text-muted-foreground'}>
          {presentation.label}
        </span>
        {presentation.meta && (
          <span className="text-[11px] text-muted-foreground">{presentation.meta}</span>
        )}
      </div>
    )
  }

  const { report } = presentation
  return (
    <details aria-label="Repository dependency check" className="group/dependencies border-b border-border">
      <summary className="flex min-h-11 cursor-pointer list-none flex-wrap items-center gap-x-3 gap-y-1 px-5 py-2.5 text-xs text-warning-strong hover:underline focus-visible:outline-2 focus-visible:outline-ring sm:px-6 lg:px-8 [&::-webkit-details-marker]:hidden">
        <ChevronRight
          aria-hidden="true"
          className="size-3.5 shrink-0 transition-transform group-open/dependencies:rotate-90"
        />
        <span>{presentation.label}</span>
        <span className="ml-auto text-[11px] text-muted-foreground no-underline">
          {presentation.meta}
        </span>
      </summary>
      <div className="px-5 pb-4 pl-12 text-xs sm:px-6 sm:pl-14 lg:px-8 lg:pl-16">
        {report.findings.length > 0 && (
          <>
            <p className="mb-2 text-muted-foreground">
              People with public access cannot read the files on the right.
            </p>
            <ul aria-label="Public files importing private files" className="max-h-72 max-w-4xl overflow-y-auto border-b border-border">
              {report.findings.map((finding, index) => (
                <li
                  className="grid grid-cols-[minmax(0,1fr)_1rem_minmax(0,1fr)] items-center gap-2 border-t border-border py-2.5 sm:gap-4"
                  key={`${finding.source_path}\0${finding.target_path}\0${index}`}
                >
                  <DependencyPath
                    label="Public file"
                    onSelectFilePath={onSelectFilePath}
                    path={finding.source_path}
                  />
                  <span aria-label="imports" className="text-center text-muted-foreground">→</span>
                  <DependencyPath
                    label="Private file"
                    onSelectFilePath={onSelectFilePath}
                    path={finding.target_path}
                  />
                </li>
              ))}
            </ul>
          </>
        )}
        {presentation.gaps.length > 0 && (
          <div className={report.findings.length > 0 ? 'mt-3' : undefined}>
            <p className="font-medium text-foreground">Coverage gaps</p>
            <ul aria-label="Dependency check coverage gaps" className="mt-1 max-h-48 max-w-4xl overflow-y-auto">
              {presentation.gaps.map((gap, index) => (
                <li className="border-t border-border py-2 text-muted-foreground" key={`${gap.path}\0${index}`}>
                  {gap.path === '.' ? (
                    <span className="mr-2 font-medium text-foreground">Repository</span>
                  ) : (
                    <button
                      className="mr-2 break-all rounded text-left font-mono text-foreground hover:underline focus-visible:outline-2 focus-visible:outline-ring"
                      onClick={() => onSelectFilePath(gap.path)}
                      type="button"
                    >
                      {gap.path}
                    </button>
                  )}
                  <span>{gap.reason}</span>
                </li>
              ))}
            </ul>
          </div>
        )}
        <p className="mt-3 max-w-4xl border-t border-border pt-2.5 text-[11px] leading-4 text-muted-foreground">
          {presentation.coverage}
        </p>
      </div>
    </details>
  )
}

function DependencyPath({
  label,
  onSelectFilePath,
  path,
}: {
  label: string
  onSelectFilePath: (path: string) => void
  path: string
}) {
  return (
    <button
      className="min-w-0 rounded text-left font-mono hover:underline focus-visible:outline-2 focus-visible:outline-ring"
      onClick={() => onSelectFilePath(path)}
      type="button"
    >
      <span className="mb-0.5 block font-sans text-[10px] text-muted-foreground">{label}</span>
      <span className="block break-all">{path}</span>
    </button>
  )
}
