import { loadAuthenticatedAccountForRequest } from '@/api/profile'
import { createServerFn } from '@tanstack/react-start'

export const loadAccountSession = createServerFn({ method: 'GET' }).handler(
  loadAuthenticatedAccountForRequest,
)
