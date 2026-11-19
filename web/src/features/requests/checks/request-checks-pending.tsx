import { TextSkeleton } from '@/components/ui/text-skeleton'

export const CHECKS_SECTION_CLASS = 'px-(--app-inset) py-2'

export function RequestChecksPending() {
  return (
    <section aria-busy="true" aria-label="CI" className={CHECKS_SECTION_CLASS}>
      <div className="border-t border-border pt-2.5">
        <TextSkeleton className="h-6 py-1.5" length="long" size="meta" />
      </div>
    </section>
  )
}
