export function fetchRefspec(branch, localRef) {
  return `+refs/heads/${branch}:${localRef}`;
}

export function readerRef(agentIndex, readerIndex, position) {
  return `refs/bench/${agentIndex}-${readerIndex}-${position}`;
}

export async function fetchedTipError(run, localRef, expectedOid) {
  const tip = (await run(['rev-parse', '--verify', '-q', localRef])).stdout;
  if (tip && (await run(['merge-base', '--is-ancestor', expectedOid, localRef])).ok) return null;
  return `stale tip: fetched ${tip ? tip.slice(0, 12) : 'nothing'}, expected ${expectedOid.slice(0, 12)} or a descendant`;
}
