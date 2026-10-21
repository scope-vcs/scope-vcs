import type { ViewsTransition } from '@/api/types.generated'
import { viewsTransitionChanges } from './views-transition-model'

export function ViewsChanges({ transition }: { transition: ViewsTransition | null }) {
  const changes = transition ? viewsTransitionChanges(transition) : []
  if (changes.length === 0) return null
  return (
    <section aria-label="Views changes" className="border-b border-border">
      <h2 className="px-(--app-inset) pt-3 pb-1 text-sm font-medium">Views changes</h2>
      <ul className="divide-y divide-border">
        {changes.map((change) => (
          <li className="px-(--app-inset) py-2 text-xs" key={change}>{change}</li>
        ))}
      </ul>
    </section>
  )
}
