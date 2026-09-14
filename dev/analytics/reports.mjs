// These queries use event identities, never mutable person profiles. A request
// can be submitted and merged by different people without breaking its funnel.
export function buildProductReports({ excludedUserIds = [], excludedRepositoryIds = [] } = {}) {
  const exclusions = [
    exclusion('distinct_id', excludedUserIds, /^scope_usr_[A-Za-z0-9_-]+$/),
    exclusion("coalesce(properties.repository_id, '')", excludedRepositoryIds, /^repoi_[A-Za-z0-9_-]+$/),
  ].filter(Boolean).join('\n    ')
  const events = `WITH product_events AS (
  SELECT event, timestamp, distinct_id, properties
  FROM events
  WHERE properties.environment = 'production'
    ${exclusions}
)`
  const report = (key, name, description, sql) => ({
    key, name, description,
    query: { kind: 'DataTableNode', source: { kind: 'HogQLQuery', query: `${events}${sql}` } },
  })

  return [
    report('activation', 'Scope: account to first push',
      'Users created 7–37 days ago; CLI authentication and first push must occur in order within 7 days of account creation. Latest 7 days excluded so the cohort has a full conversion window.', `,
  signups AS (
    SELECT distinct_id AS actor, min(timestamp) AS created
    FROM product_events WHERE event = 'account:user_create'
    GROUP BY actor HAVING created >= now() - INTERVAL 37 DAY AND created < now() - INTERVAL 7 DAY
  ), sessions AS (
    SELECT s.actor AS actor, s.created AS created, min(e.timestamp) AS authenticated
    FROM signups s JOIN product_events e ON s.actor = e.distinct_id
    WHERE e.event = 'cli:session_create' AND e.timestamp >= s.created
      AND e.timestamp <= s.created + INTERVAL 7 DAY
    GROUP BY s.actor, s.created
  ), initialized AS (
    SELECT s.actor AS actor FROM sessions s JOIN product_events e ON s.actor = e.distinct_id
    WHERE e.event = 'repository:repository_initialize' AND e.timestamp >= s.authenticated
      AND e.timestamp <= s.created + INTERVAL 7 DAY GROUP BY s.actor
  )
SELECT * FROM (
SELECT 1 AS step, 'Account created' AS outcome, count() AS users FROM signups
UNION ALL SELECT 2, 'CLI authenticated', count() FROM sessions
UNION ALL SELECT 3, 'First push completed', count() FROM initialized
) ORDER BY step`),
    report('request-completion', 'Scope: request completion',
      'Requests first submitted in the past 30 days, correlated by request ID across actors. Includes open requests in the denominator. Durations cover observed discussion/merge outcomes only; discussion is optional for merging.', `,
  submitted AS (
    SELECT properties.request_id AS request_id, min(timestamp) AS submitted_at
    FROM product_events WHERE event = 'request:request_submit' AND notEmpty(properties.request_id)
    GROUP BY request_id HAVING submitted_at >= now() - INTERVAL 30 DAY
  ), outcomes AS (
    SELECT s.request_id AS request_id, s.submitted_at AS submitted_at,
      countIf(e.event = 'discussion:discussion_create') > 0 AS discussed,
      countIf(e.event = 'request:request_merge') > 0 AS merged,
      minIf(e.timestamp, e.event = 'discussion:discussion_create') AS discussed_at,
      minIf(e.timestamp, e.event = 'request:request_merge') AS merged_at
    FROM submitted s LEFT JOIN product_events e ON s.request_id = e.properties.request_id
    WHERE e.timestamp >= s.submitted_at
    GROUP BY s.request_id, s.submitted_at
  )
SELECT count() AS submitted_requests, countIf(discussed) AS discussed_requests,
  countIf(merged) AS merged_requests,
  round(100.0 * countIf(merged) / nullIf(count(), 0), 1) AS merge_percent,
  avgIf(dateDiff('second', submitted_at, discussed_at), discussed) AS mean_seconds_to_discussion,
  avgIf(dateDiff('second', submitted_at, merged_at), merged) AS mean_seconds_to_merge
FROM outcomes`),
    report('repository-retention', 'Scope: weekly active and returning repositories',
      'Repository identity is the opaque incarnation ID. Activity means initialization, push, request submit/revision/merge, or discussion creation/reply. Returning means also active in the immediately preceding week. Current partial week excluded.', `,
  activity AS (
    SELECT toStartOfWeek(timestamp, 1) AS week, properties.repository_id AS repository_id
    FROM product_events
    WHERE event IN ('repository:repository_initialize', 'repository:push_complete',
      'request:request_submit', 'request:revision_create', 'request:request_merge',
      'discussion:discussion_create', 'discussion:reply_create')
      AND notEmpty(properties.repository_id)
      AND timestamp >= toStartOfWeek(now(), 1) - INTERVAL 13 WEEK
      AND timestamp < toStartOfWeek(now(), 1)
    GROUP BY week, repository_id
  )
SELECT a.week AS week, count() AS active_repositories,
  countIf(notEmpty(p.repository_id)) AS returning_repositories,
  round(100.0 * countIf(notEmpty(p.repository_id)) / nullIf(count(), 0), 1) AS returning_share_percent
FROM activity a LEFT JOIN activity p
  ON a.repository_id = p.repository_id AND a.week = p.week + INTERVAL 1 WEEK
WHERE a.week >= toStartOfWeek(now(), 1) - INTERVAL 12 WEEK
GROUP BY a.week ORDER BY a.week`),
    report('contributors', 'Scope: weekly active and returning contributors',
      'Users performing pushes, request work or discussion work. Returning contributors had an earlier observed activity week. Excludes the current partial week and system actors.', `,
  activity AS (
    SELECT toStartOfWeek(timestamp, 1) AS week, distinct_id AS actor
    FROM product_events WHERE event IN ('repository:repository_initialize', 'repository:push_complete',
      'request:request_submit', 'request:revision_create', 'request:request_merge',
      'discussion:discussion_create', 'discussion:reply_create')
      AND startsWith(distinct_id, 'scope_usr_')
      AND timestamp < toStartOfWeek(now(), 1)
    GROUP BY week, actor
  ), first_seen AS (SELECT actor, min(week) AS first_week FROM activity GROUP BY actor)
SELECT a.week AS week, count() AS active_contributors,
  countIf(f.first_week < a.week) AS returning_contributors
FROM activity a JOIN first_seen f ON a.actor = f.actor
WHERE a.week >= toStartOfWeek(now(), 1) - INTERVAL 12 WEEK
GROUP BY a.week ORDER BY a.week`),
    report('workflow-attempts', 'Scope: workflow attempt outcomes',
      'Terminal attempts in the last 30 days, deduplicated by attempt ID. Duration is per attempt, including provisioning. Retry attempts count separately here.', `,
  attempts AS (
    SELECT properties.attempt_id AS attempt_id,
      argMax(properties.result, timestamp) AS result,
      argMax(toFloat(properties.duration_ms), timestamp) AS duration_ms
    FROM product_events WHERE event = 'workflow:attempt_complete'
      AND timestamp >= now() - INTERVAL 30 DAY AND notEmpty(properties.attempt_id)
    GROUP BY attempt_id
  )
SELECT result, count() AS attempts, avg(duration_ms) AS mean_duration_ms
FROM attempts GROUP BY result ORDER BY result`),
    report('workflow-runs', 'Scope: completed workflow run outcomes',
      'Only completion events carrying a terminal overall run_result count. A successful intermediate job does not mean the run succeeded. Deduplicated by run ID; durations cover the first observed admitted attempt to terminal completion.', `,
  starts AS (
    SELECT properties.run_id AS run_id, min(timestamp) AS started_at
    FROM product_events WHERE event = 'workflow:attempt_start' AND notEmpty(properties.run_id)
    GROUP BY run_id
  ), completed AS (
    SELECT properties.run_id AS run_id, argMax(properties.run_result, timestamp) AS result,
      max(timestamp) AS completed_at
    FROM product_events WHERE event = 'workflow:attempt_complete' AND notEmpty(properties.run_result)
      AND notEmpty(properties.run_id) GROUP BY run_id
  )
SELECT c.result AS result, count() AS completed_runs,
  avgIf(dateDiff('second', s.started_at, c.completed_at), notEmpty(s.run_id)) AS mean_run_seconds
FROM completed c LEFT JOIN starts s ON c.run_id = s.run_id
WHERE c.completed_at >= now() - INTERVAL 30 DAY GROUP BY c.result ORDER BY c.result`),
  ]
}

function exclusion(property, ids, pattern) {
  if (!Array.isArray(ids) || ids.some((id) => typeof id !== 'string' || !pattern.test(id))) {
    throw new Error(`Invalid opaque IDs for ${property}`)
  }
  return ids.length ? `AND ${property} NOT IN (${[...new Set(ids)].map((id) => `'${id}'`).join(', ')})` : ''
}
