import { createCachedResource } from '@/lib/cached-resource'
import { useCachedResource } from '@/lib/use-cached-resource'
import { useCallback, useRef, useState } from 'react'
import type { RequestRevisionListResponse } from '@/api/types.generated'
import { requestChangesResource, requestChangesSelectionIdentity } from '@/features/requests/request-changes-resource'

export function ChangesResourceFixture() {
  const [viewerId, setViewerId] = useState('user_clerk_original')
  const [access, setAccess] = useState('private')
  const [opened, setOpened] = useState(true)
  const [loads, setLoads] = useState(0)
  const identity = requestChangesSelectionIdentity(`${viewerId}:${access}`, 'request-fixture')
  const load = useCallback(async () => {
    setLoads(count => count + 1)
    return { comparison: null, has_earlier_revisions: false, revisions: [], review_revision_id: `${viewerId}:${access}` } satisfies RequestRevisionListResponse
  }, [access, viewerId])
  return <section>
    <h2>Changes resource</h2>
    <output aria-label="Revision loads">{loads}</output>
    <button onClick={() => setOpened(value => !value)}>Toggle changes</button>
    <button onClick={() => requestChangesResource.invalidate(identity)}>Invalidate changes</button>
    <button onClick={() => setViewerId('user_clerk_other')}>Switch changes viewer</button>
    <button onClick={() => setAccess('public')}>Switch changes access</button>
    {opened && <Changes identity={identity} load={load} />}
  </section>
}

function Changes({ identity, load }: { identity: string; load: () => Promise<RequestRevisionListResponse> }) {
  const resource = useCachedResource({ fallbackError: 'Request changes are unavailable.', identity, load, resource: requestChangesResource })
  return <output aria-label="Selected revision">{resource.value?.review_revision_id}</output>
}

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
