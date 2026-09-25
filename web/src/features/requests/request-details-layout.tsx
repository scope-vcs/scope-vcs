import { TextSkeleton } from '@/components/ui/skeleton'
import type { ReactNode } from 'react'

export function DetailsSection({
  children,
  title,
}: {
  children: ReactNode
  title: string
}) {
  return (
    <section>
      <h2 className="label-mono text-muted-foreground">{title}</h2>
      <div className="mt-3 grid min-w-0 gap-2.5">{children}</div>
    </section>
  )
}

export function DetailsValue({ label, value }: { label: string; value: ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 text-[13px]">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 break-all text-right font-mono">{value}</span>
    </div>
  )
}

/** RequestDetails before the request loads: its fixed sections and labels. */
export function RequestDetailsSkeleton() {
  return (
    <div className="@container min-w-0">
      <section className="min-w-0 px-5 py-6 @md:px-6 @3xl:px-8">
        <div className="grid min-w-0 gap-x-12 gap-y-8 @3xl:grid-cols-2">
          <DetailsSection title="lifecycle">
            {['Author', 'Audience', 'Submitted'].map((label) => (
              <DetailsValue key={label} label={label} value={<TextSkeleton className="ml-auto" length="short" size="meta" />} />
            ))}
          </DetailsSection>
          <DetailsSection title="git state">
            {['Base', 'Head'].map((label) => (
              <DetailsValue key={label} label={label} value={<TextSkeleton className="ml-auto" length="short" size="meta" />} />
            ))}
          </DetailsSection>
        </div>
      </section>
    </div>
  )
}
