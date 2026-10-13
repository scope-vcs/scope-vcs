import { TextSkeleton } from '@/components/ui/skeleton'

export const CHECKS_SECTION_CLASS = 'border-b border-border px-(--app-inset) py-4'

export function RequestChecksPending() {
  return (
    <section aria-busy="true" aria-label="CI" className={CHECKS_SECTION_CLASS}>
      <h2 className="label-mono text-muted-foreground">CI</h2>
      <TextSkeleton className="mt-2 h-5" length="xlong" size="meta" />
    </section>
  )
}
