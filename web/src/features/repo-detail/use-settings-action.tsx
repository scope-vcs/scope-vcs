import { PageErrorAlert } from '@/components/page-error-alert'
import { resourceErrorMessage } from '@/lib/use-cached-resource'
import { useState } from 'react'

export function useSettingsAction<Key extends string>() {
  const [pending, setPending] = useState<Key | null>(null)
  const [error, setError] = useState<{ title: string; message: string } | null>(null)

  async function act(key: Key, title: string, action: () => Promise<unknown>, leavesPage = false) {
    setError(null)
    setPending(key)
    try {
      await action()
      if (!leavesPage) setPending(null)
    } catch (cause) {
      setPending(null)
      setError({ title, message: resourceErrorMessage(cause, 'Try again.') })
    }
  }

  const errorAlert = error && (
    <PageErrorAlert className="mt-0" title={error.title}>{error.message}</PageErrorAlert>
  )
  return { act, errorAlert, pending }
}
