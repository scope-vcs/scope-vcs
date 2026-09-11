export const repository = 'scope-vcs/scope-vcs';
export const sourceSha = 'a'.repeat(40);
export const backendComponents = ['api', 'run-worker', 'cache', 'git-router', 'media-api', 'media-worker'];

export function preparedRelease({ components = backendComponents, services = {}, sourceRunId = '123' } = {}) {
  return {
    schemaVersion: 1, sourceSha, preparationRunId: sourceRunId, maintenanceSha256: 'c'.repeat(64),
    components: Object.fromEntries(components.map(component => [component, {
      sourceSha, serviceId: services[component]?.id ?? component,
      image: component === 'media-worker'
        ? `ghcr.io/scope-vcs/scope-media-worker@sha256:${'b'.repeat(64)}`
        : `ghcr.io/${repository}/railway-private-${({ 'run-worker': 'worker', 'git-router': 'router', 'media-api': 'media' })[component] ?? component}@sha256:${'b'.repeat(64)}`,
    }])),
  };
}

// A successful preparation from a failed release is sufficient for recovery.
// Replay tests add the separate validation and staging evidence they require.
export function releaseFixture({ mainSha = 'd'.repeat(40), ...options } = {}) {
  const prepared = preparedRelease(options);
  const runId = prepared.preparationRunId;
  const run = {
    id: Number(runId), path: '.github/workflows/release.yml', event: 'schedule',
    head_branch: 'main', head_sha: sourceSha, status: 'completed', conclusion: 'failure',
    repository: { id: 1, full_name: repository }, head_repository: { id: 1, full_name: repository },
  };
  const main = { name: 'main', commit: { sha: mainSha } };
  const comparison = { status: 'ahead', base_commit: { sha: sourceSha }, merge_base_commit: { sha: sourceSha } };
  const jobs = [{
    id: 456, run_id: Number(runId), head_sha: sourceSha, name: 'Prepare Railway artifacts / prepare',
    status: 'completed', conclusion: 'success',
    steps: [{ name: 'Prepare immutable release images', status: 'completed', conclusion: 'success' }],
  }];
  const calls = [];
  const request = async path => {
    calls.push(path);
    if (path === `/actions/runs/${runId}`) return structuredClone(run);
    if (path === '/branches/main') return structuredClone(main);
    if (path === `/compare/${sourceSha}...${mainSha}`) return structuredClone(comparison);
    const match = /^\/actions\/runs\/(\d+)\/jobs\?filter=all&per_page=100&page=(\d+)$/.exec(path);
    if (match && match[1] === runId) {
      const page = Number(match[2]);
      return { jobs: structuredClone(jobs.slice((page - 1) * 100, page * 100)) };
    }
    throw new Error(`Unexpected request ${path}`);
  };
  return { prepared, run, main, comparison, jobs, calls, request };
}
