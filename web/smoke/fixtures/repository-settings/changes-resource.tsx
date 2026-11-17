import { createCachedResource } from '@/lib/cached-resource'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useCallback, useRef, useState } from 'react'

const coalescedResource = createCachedResource<{ value: string }>({ maxEntries: 1, coalesceInvalidations: true })

export function CoalescedResourceFixture() {
  const [loads, setLoads] = useState(0)
  const pending = useRef<Array<() => void>>([])
  const load = useCallback(() => new Promise<{ value: string }>(resolve => {
    setLoads(count => count + 1)
    pending.current.push(() => resolve({ value: 'Updated snapshot' }))
  }), [])
  const resource = useCachedResource({
    fallbackError: 'Snapshot failed', identity: 'coalesced', initialValue: initialCoalesced,
    load, resource: coalescedResource,
  })
  return <section>
    <h2>Coalesced refresh</h2>
    <output aria-label="Snapshot loads">{loads}</output>
    <output aria-label="Snapshot value">{resource.value?.value}</output>
    <output aria-label="Snapshot refreshing">{String(resource.refreshing)}</output>
    <button onClick={() => coalescedResource.invalidate('coalesced')}>Invalidate snapshot</button>
    <button onClick={() => pending.current.shift()?.()}>Finish snapshot</button>
  </section>
}

const initialCoalesced = { value: 'Retained snapshot' }
