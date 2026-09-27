import {
  loadHistoryEntryFileDiffForRequest,
  loadHistoryEntryForRequest,
  loadHistoryPageForRequest,
  parseHistoryEntryDetailInput,
  parseHistoryEntryFileDiffInput,
  parseHistoryPageInput,
} from '@/api/history'
import { isNotFoundError } from '@/api/http'
import { notFound } from '@tanstack/react-router'
import { createServerFn } from '@tanstack/react-start'
import { getRequest } from '@tanstack/react-start/server'
import { auth } from '@clerk/tanstack-react-start/server'

export const loadHistoryPage = createServerFn({ method: 'GET' })
  .validator(parseHistoryPageInput)
  .handler(({ data }) => loadHistoryPageForRequest(data))

export const loadHistoryEntry = createServerFn({ method: 'GET' })
  .validator(parseHistoryEntryDetailInput)
  .handler(async ({ data }) => {
    try {
      const [{ userId }, entry] = await Promise.all([auth(), loadHistoryEntryForRequest(data)])
      return { entry, viewerId: userId }
    } catch (error) {
      if (isNotFoundError(error)) throw notFound()
      throw error
    }
  })

export const loadHistoryEntryFileDiff = createServerFn({ method: 'GET' })
  .validator(parseHistoryEntryFileDiffInput)
  .handler(({ data }) => loadHistoryEntryFileDiffForRequest(data, getRequest().signal))
