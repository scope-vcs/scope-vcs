import { useCallback, useRef, useState } from 'react'

// Keep independent rows interactive without allowing duplicate work on one row.
export function usePendingActions() {
  const active = useRef(new Set<string>())
  const [pending, setPending] = useState<ReadonlySet<string>>(() => new Set())
  const run = useCallback(async (key: string, action: () => Promise<void>) => {
    if (active.current.has(key)) return
    active.current.add(key)
    setPending(new Set(active.current))
    try {
      await action()
    } finally {
      active.current.delete(key)
      setPending(new Set(active.current))
    }
  }, [])
  return { pending, run }
}
