const TARGET_KINDS = new Set(['loadtest', 'staging', 'preview']);
const PREVIEW_LABEL = /^(?:.+-)?pr-\d+(?:-.+)?$/;

export function validateTargetKind(kind) {
  if (!TARGET_KINDS.has(kind)) {
    throw new Error('SCOPE_BENCH_TARGET_KIND must be loadtest, staging, or preview');
  }
}

export function assertSafeTarget(target, kind = 'loadtest') {
  validateTargetKind(kind);
  const url = new URL(target);
  const local = ['localhost', '127.0.0.1', '::1'].includes(url.hostname);
  if (!local && !hostnameMatchesKind(url.hostname.toLowerCase(), kind)) {
    throw new Error(`refusing non-${kind} target: ${url.hostname}`);
  }
}

function hostnameMatchesKind(hostname, kind) {
  const labels = hostname.split('.');
  if (kind === 'preview') {
    return labels.some((label) => PREVIEW_LABEL.test(label)) && !labels.flatMap((label) => label.split('-')).includes('production');
  }
  return labels.flatMap((label) => label.split('-')).includes(kind);
}
