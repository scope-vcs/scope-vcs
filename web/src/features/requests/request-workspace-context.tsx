import { createContext, use, type ReactNode } from 'react'
import type { RequestQueueItemResponse } from '@/api/types.generated'

type RequestWorkspaceContextValue = {
  selected: RequestQueueItemResponse | null
  previousId: string | null
  nextId: string | null
  claim: () => void
}

const RequestWorkspaceContext = createContext<RequestWorkspaceContextValue | null>(null)

export function RequestWorkspaceProvider({ children, value }: { children: ReactNode; value: RequestWorkspaceContextValue }) {
  return <RequestWorkspaceContext value={value}>{children}</RequestWorkspaceContext>
}

export function useRequestWorkspace() {
  return use(RequestWorkspaceContext)
}
