function browserSessionStorage() {
  return typeof window === 'undefined' ? null : window.sessionStorage
}

export function readAndClearSessionValue(key: string) {
  const storage = browserSessionStorage()
  if (!storage) {
    return null
  }

  const value = storage.getItem(key)
  if (value !== null) {
    storage.removeItem(key)
  }
  return value
}

export function storeSessionValue(key: string, value: string) {
  browserSessionStorage()?.setItem(key, value)
}
