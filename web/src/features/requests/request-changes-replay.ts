import type { RequestParams } from '@/api/types'

type ChangesReplayInput = RequestParams & { commit_oid?: string; revision_id?: string }

type ChangesReplay<T> = { data: T; key: string }

// Pinning a revision rewrites the URL right after the loader returned, which
// runs the loader again for the same selection. The pinned page is handed to
// that second run once so the client does not fetch it twice.
const pinnedChangesReplay: { current: ChangesReplay<unknown> | null } = { current: null }

export function rememberPinnedChangesReplay<T>(input: ChangesReplayInput, data: T) {
  if (typeof window === 'undefined') return null
  const replay: ChangesReplay<T> = { data, key: changesSelectionKey(input) }
  pinnedChangesReplay.current = replay
  return replay
}

export function takePinnedChangesReplay<T>(input: ChangesReplayInput) {
  if (typeof window === 'undefined') return null
  const replay = pinnedChangesReplay.current
  if (!replay || replay.key !== changesSelectionKey(input)) return null
  pinnedChangesReplay.current = null
  return replay.data as T
}

export function forgetPinnedChangesReplay(replay: ChangesReplay<unknown> | null) {
  if (pinnedChangesReplay.current === replay) pinnedChangesReplay.current = null
}

function changesSelectionKey(input: ChangesReplayInput) {
  return [
    input.owner,
    input.repo,
    input.request_id,
    input.revision_id ?? '',
    input.commit_oid ?? '',
  ].join('\0')
}
