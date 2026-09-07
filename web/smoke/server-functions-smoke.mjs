import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'

// TanStack Start hashes relative source paths and handler exports in production.
// Keep the read functions observed by these smoke tests explicit so a moved
// function or changed compiler ID format fails instead of bypassing interception.
const sourceFunctions = {
  'src/analytics/analytics-root.tsx': ['loadAnalyticsIdentity'],
  'src/routes/index.tsx': ['loadIndex'],
  'src/routes/$owner.$repo.tsx': ['loadRepoLiveState'],
  'src/routes/$owner.$repo._code.index.tsx': ['loadRepoContent', 'loadRepoFile'],
  'src/routes/-repo-activity-actions.ts': ['loadRepositoryLatestActivity'],
  'src/routes/-repo-history-actions.ts': ['loadHistoryPage', 'loadHistoryEntry', 'loadHistoryEntryFileDiff'],
  'src/routes/$owner.$repo.requests.index.tsx': ['loadRequestQueuePage'],
  'src/routes/$owner.$repo.requests.$requestId.tsx': ['loadRequestPage', 'loadActivity', 'listRequestAttachments', 'loadAttachmentLimits', 'prepareAttachment', 'finishAttachment', 'retryAttachment', 'grantAttachmentMedia'],
  'src/routes/$owner.$repo.requests.$requestId.index.tsx': ['loadDiscussionPage', 'loadDiscussions', 'loadReplies', 'loadDiscussionChanges'],
  'src/routes/$owner.$repo.requests.$requestId.changes.tsx': ['loadChangesPage', 'loadRevisionDiff', 'loadDiscussions'],
}

const productionFunctions = new Map(Object.entries(sourceFunctions).flatMap(([filename, names]) => (
  names.map((name) => {
    const handler = `${name}_createServerFn_handler`
    return [createHash('sha256').update(`${filename}--${handler}`).digest('hex'), handler]
  })
)))

export function serverFunctionName(request) {
  const pathname = new URL(request.url()).pathname
  if (!pathname.startsWith('/_serverFn/')) return ''
  const id = pathname.slice('/_serverFn/'.length)
  const productionName = productionFunctions.get(id)
  if (productionName) return productionName

  let developmentName
  try {
    developmentName = JSON.parse(Buffer.from(id, 'base64url').toString('utf8')).export
  } catch {}
  assert(typeof developmentName === 'string' && developmentName.length > 0,
    `Unknown server function ID ${JSON.stringify(id)}; update smoke/server-functions-smoke.mjs for the current source paths and TanStack compiler`)
  return developmentName
}
