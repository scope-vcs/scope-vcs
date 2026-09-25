import { TextSkeleton } from '@/components/ui/skeleton'

export const CHECKS_SECTION_CLASS = 'border-b border-border px-5 py-4 sm:px-6 lg:px-8'

/** The checks row while checks load, so the page below does not move. */
export function RequestChecksPending() {
  return (
    <section aria-busy="true" aria-label="Checks" className={CHECKS_SECTION_CLASS}>
      <h2 className="label-mono text-muted-foreground">checks</h2>
      <TextSkeleton className="mt-2 h-5" length="xlong" size="meta" />
    </section>
  )
}
