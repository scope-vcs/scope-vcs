import { useSyncExternalStore } from 'react'

const TICK_MS = 15_000

const listeners = new Set<() => void>()
let timer: ReturnType<typeof setInterval> | null = null
let nowUnix = currentUnix()
let readAtMs = Date.now()

function currentUnix() {
  return Math.floor(Date.now() / 1_000)
}

function tick() {
  const next = currentUnix()
  if (next === nowUnix) return
  nowUnix = next
  readAtMs = Date.now()
  for (const listener of listeners) listener()
}

function subscribe(listener: () => void) {
  if (listeners.size === 0) tick()
  listeners.add(listener)
  timer ??= setInterval(tick, TICK_MS)
  return () => {
    listeners.delete(listener)
    if (listeners.size > 0 || timer === null) return
    clearInterval(timer)
    timer = null
  }
}

function snapshot() {
  return nowUnix
}

function serverSnapshot() {
  const elapsed = Date.now() - readAtMs
  if (elapsed >= 1_000) {
    nowUnix = currentUnix()
    readAtMs = Date.now()
  }
  return nowUnix
}

export function useUnixClock() {
  return useSyncExternalStore(subscribe, snapshot, serverSnapshot)
}
