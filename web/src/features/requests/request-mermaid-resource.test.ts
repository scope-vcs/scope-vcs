import assert from 'node:assert/strict'
import test from 'node:test'
import {
  createRequestMermaidResourceManager,
  requestMermaidIdentity,
  type RequestMermaidInput,
  type RequestMermaidResult,
} from './request-mermaid-resource'

const input = (
  source: string,
  theme: 'light' | 'dark' = 'light',
  accessScope = 'scope',
): RequestMermaidInput => ({ accessScope, source, theme })

const result = (svg: string): RequestMermaidResult => ({ svg, width: 100, height: 50 })

test('deduplicates active renders and reuses the cached result after reopening', async () => {
  const calls: RequestMermaidInput[] = []
  const manager = createRequestMermaidResourceManager({
    render: async (renderInput) => {
      calls.push({ accessScope: 'scope', ...renderInput })
      return result(`<svg>${renderInput.source}</svg>`)
    },
    yieldToBrowser: async () => {},
  })
  const current = input('graph TD; A-->B')
  const firstRelease = manager.acquire(current, 1)
  const secondRelease = manager.acquire(current, 0)

  await completed(manager, current)
  assert.equal(calls.length, 1)
  firstRelease()
  secondRelease()

  const reopenRelease = manager.acquire(current, 0)
  await flush()
  assert.equal(calls.length, 1)
  assert.match(manager.resource.getSnapshot(requestMermaidIdentity(current)).value?.svg ?? '', /A-->B/)
  reopenRelease()
})

test('source, theme, and access scope are independent exact cache identities', async () => {
  const calls: Array<{ source: string; theme: 'light' | 'dark' }> = []
  const manager = createRequestMermaidResourceManager({
    render: async (renderInput) => {
      calls.push(renderInput)
      return result(`<svg>${calls.length}</svg>`)
    },
    yieldToBrowser: async () => {},
  })
  const variants = [
    input('graph TD; A-->B'),
    input('graph TD; A-->C'),
    input('graph TD; A-->B', 'dark'),
    input('graph TD; A-->B', 'light', 'other-scope'),
  ]

  for (const variant of variants) {
    const release = manager.acquire(variant, 0)
    await completed(manager, variant)
    release()
  }

  assert.equal(new Set(variants.map(requestMermaidIdentity)).size, variants.length)
  assert.equal(calls.length, variants.length)
})

test('runs one render at a time, yields between renders, and prioritizes visible work', async () => {
  const blocker = deferred<RequestMermaidResult>()
  const visible = deferred<RequestMermaidResult>()
  const nearby = deferred<RequestMermaidResult>()
  const yieldGate = deferred<void>()
  const calls: string[] = []
  let yields = 0
  const manager = createRequestMermaidResourceManager({
    render: (renderInput) => {
      calls.push(renderInput.source)
      if (renderInput.source === 'blocker') return blocker.promise
      if (renderInput.source === 'visible') return visible.promise
      return nearby.promise
    },
    yieldToBrowser: () => {
      yields++
      return yields === 1 ? yieldGate.promise : Promise.resolve()
    },
  })
  const releases = [
    manager.acquire(input('blocker'), 1),
    manager.acquire(input('nearby'), 1),
    manager.acquire(input('visible'), 0),
  ]
  await flush()
  assert.deepEqual(calls, ['blocker'])
  assert.equal(manager.resource.stats().entries, 1, 'waiting diagrams do not allocate pending cache entries')

  blocker.resolve(result('<svg>blocker</svg>'))
  await flush()
  assert.deepEqual(calls, ['blocker'])
  yieldGate.resolve()
  await flush()
  assert.deepEqual(calls, ['blocker', 'visible'])

  visible.resolve(result('<svg>visible</svg>'))
  await flush()
  assert.deepEqual(calls, ['blocker', 'visible', 'nearby'])
  nearby.resolve(result('<svg>nearby</svg>'))
  await flush()
  for (const release of releases) release()
})

test('canceling the final lease removes waiting work before it can render', async () => {
  const blocker = deferred<RequestMermaidResult>()
  const calls: string[] = []
  const manager = createRequestMermaidResourceManager({
    render: (renderInput) => {
      calls.push(renderInput.source)
      return renderInput.source === 'blocker'
        ? blocker.promise
        : Promise.resolve(result('<svg>unexpected</svg>'))
    },
    yieldToBrowser: async () => {},
  })
  const releaseBlocker = manager.acquire(input('blocker'), 0)
  const canceled = input('cancel me')
  const releaseCanceled = manager.acquire(canceled, 1)
  releaseCanceled()

  assert.equal(manager.resource.getSnapshot(requestMermaidIdentity(canceled)).pending, false)
  blocker.resolve(result('<svg>blocker</svg>'))
  await flush()
  assert.deepEqual(calls, ['blocker'])
  assert.equal(manager.resource.peek(requestMermaidIdentity(canceled)), null)
  releaseBlocker()
})

test('access changes discard old-scope work and suppress its late result', async () => {
  const oldRender = deferred<RequestMermaidResult>()
  const calls: string[] = []
  const previousScope = JSON.stringify(['repo', 'viewer', { role: 'Reader' }])
  const nextScope = JSON.stringify(['repo', 'viewer', { role: 'Maintainer' }])
  const previous = input('old', 'light', previousScope)
  const next = input('new', 'light', nextScope)
  const manager = createRequestMermaidResourceManager({
    render: (renderInput) => {
      calls.push(renderInput.source)
      return renderInput.source === 'old'
        ? oldRender.promise
        : Promise.resolve(result('<svg>new</svg>'))
    },
    yieldToBrowser: async () => {},
  })

  manager.activateScope(previousScope)
  manager.acquire(previous, 0)
  await flush()
  manager.activateScope(nextScope)
  const releaseNext = manager.acquire(next, 0)
  oldRender.resolve(result('<svg>obsolete</svg>'))
  await completed(manager, next)

  assert.deepEqual(calls, ['old', 'new'])
  assert.equal(manager.resource.peek(requestMermaidIdentity(previous)), null)
  assert.equal(manager.resource.peek(requestMermaidIdentity(next))?.svg, '<svg>new</svg>')
  releaseNext()
})

test('reset discards waiting work and late results without overlapping renders', async () => {
  const obsolete = deferred<RequestMermaidResult>()
  const calls: string[] = []
  const current = input('reopened')
  const manager = createRequestMermaidResourceManager({
    render: async ({ source }) => {
      calls.push(source)
      return calls.length === 1 ? obsolete.promise : result('<svg>fresh</svg>')
    },
    yieldToBrowser: async () => {},
  })
  const releaseOld = manager.acquire(current, 0)
  manager.acquire(input('discard waiting'), 0)
  await flush()
  manager.reset()
  const releaseNew = manager.acquire(current, 0)
  releaseOld()
  releaseOld()
  await flush()
  assert.deepEqual(calls, ['reopened'], 'reset keeps the active renderer occupied')
  assert.equal(manager.resource.peek(requestMermaidIdentity(current)), null)

  obsolete.resolve(result('<svg>obsolete</svg>'))
  await completed(manager, current)
  assert.deepEqual(calls, ['reopened', 'reopened'])
  assert.equal(manager.resource.peek(requestMermaidIdentity(current))?.svg, '<svg>fresh</svg>')
  releaseNew()
})

test('reopening refreshes retention recency and accounts for source plus SVG weight', async () => {
  const manager = createRequestMermaidResourceManager({
    render: async (renderInput) => result(`<svg>${renderInput.source}</svg>`),
    yieldToBrowser: async () => {},
    maxEntries: 2,
  })
  const first = input('first')
  const second = input('second')
  const third = input('third')
  for (const current of [first, second]) {
    const release = manager.acquire(current, 0)
    await completed(manager, current)
    release()
  }
  manager.acquire(first, 0)()
  const releaseThird = manager.acquire(third, 0)
  await completed(manager, third)
  releaseThird()

  assert.notEqual(manager.resource.peek(requestMermaidIdentity(first)), null)
  assert.equal(manager.resource.peek(requestMermaidIdentity(second)), null)
  assert.notEqual(manager.resource.peek(requestMermaidIdentity(third)), null)

  const weighted = createRequestMermaidResourceManager({
    render: async () => result('1234'),
    yieldToBrowser: async () => {},
    maxWeight: 13,
  })
  const oversized = input('abc')
  const releaseOversized = weighted.acquire(oversized, 0)
  await flush()
  assert.deepEqual(weighted.resource.stats(), { entries: 0, totalWeight: 0 })
  releaseOversized()
})

test('completed active demands do not rerender when the bounded cache evicts them', async () => {
  const calls: string[] = []
  const manager = createRequestMermaidResourceManager({
    render: async (renderInput) => {
      calls.push(renderInput.source)
      return result(`<svg>${renderInput.source}</svg>`)
    },
    yieldToBrowser: async () => {},
    maxEntries: 1,
  })
  const releases = ['first', 'second', 'third'].map((source) => (
    manager.acquire(input(source), 1)
  ))

  await waitFor(() => calls.length === 3)
  await flush()
  assert.deepEqual(calls, ['first', 'second', 'third'])
  for (const release of releases) release()
})

test('a failed render waits for explicit retry and retry requires an active lease', async () => {
  let calls = 0
  const current = input('retry')
  const manager = createRequestMermaidResourceManager({
    render: async () => {
      calls++
      if (calls === 1) throw new Error('invalid diagram')
      return result('<svg>recovered</svg>')
    },
    yieldToBrowser: async () => {},
  })
  const release = manager.acquire(current, 0)
  await failed(manager, current)
  await flush()
  assert.equal(calls, 1)

  manager.retry(current, 0)
  await completed(manager, current)
  assert.equal(calls, 2)
  release()
  manager.resource.invalidate(requestMermaidIdentity(current))
  manager.retry(current, 0)
  await flush()
  assert.equal(calls, 2)
})

test('retrying a started render suppresses its late result and queues one replacement', async () => {
  const first = deferred<RequestMermaidResult>()
  let calls = 0
  const current = input('retry while rendering')
  const manager = createRequestMermaidResourceManager({
    render: async () => {
      calls++
      if (calls === 1) return first.promise
      return result('<svg>replacement</svg>')
    },
    yieldToBrowser: async () => {},
  })
  const release = manager.acquire(current, 0)
  await flush()
  manager.retry(current, 0)
  first.resolve(result('<svg>obsolete</svg>'))

  await completed(manager, current)
  assert.equal(calls, 2)
  assert.equal(
    manager.resource.peek(requestMermaidIdentity(current))?.svg,
    '<svg>replacement</svg>',
  )
  release()
})

async function completed(
  manager: ReturnType<typeof createRequestMermaidResourceManager>,
  current: RequestMermaidInput,
) {
  await waitFor(() => {
    const snapshot = manager.resource.getSnapshot(requestMermaidIdentity(current))
    return snapshot.value !== null && !snapshot.pending
  })
}

async function failed(
  manager: ReturnType<typeof createRequestMermaidResourceManager>,
  current: RequestMermaidInput,
) {
  await waitFor(() => manager.resource.getSnapshot(requestMermaidIdentity(current)).error !== null)
}

async function waitFor(predicate: () => boolean) {
  for (let attempt = 0; attempt < 50; attempt++) {
    if (predicate()) return
    await Promise.resolve()
  }
  assert.fail('condition did not settle')
}

async function flush() {
  for (let turn = 0; turn < 10; turn++) await Promise.resolve()
}

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((nextResolve, nextReject) => {
    resolve = nextResolve
    reject = nextReject
  })
  return { promise, reject, resolve }
}
