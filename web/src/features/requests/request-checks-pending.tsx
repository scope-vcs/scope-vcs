import { TextSkeleton } from '@/components/ui/skeleton'

export const CHECKS_SECTION_CLASS = 'px-5 pt-4 pb-2 lg:px-7'

export function RequestChecksPending() {
  return (
    <section aria-busy="true" aria-label="CI" className={CHECKS_SECTION_CLASS}>
      <h2 className="section-label">CI</h2>
      <TextSkeleton className="mt-2 h-5" length="xlong" size="meta" />
    </section>
  )
}
