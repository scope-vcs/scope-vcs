import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function baselineIdentity(plan) {
  assert(Array.isArray(plan?.applied) && plan.applied.length > 0,
    'Production baseline must contain a nonempty applied migration ledger');
  assert(plan.applied.every((name) => typeof name === 'string' && /^m[0-9]+_[a-z0-9_]+$/.test(name)),
    'Invalid applied migration ledger');
  assert.equal(new Set(plan.applied).size, plan.applied.length, 'Duplicate applied migration');
  return createHash('sha256').update(JSON.stringify(plan.applied)).digest('hex');
}

export function sameBaseline(production, staging) {
  return baselineIdentity(production) === baselineIdentity(staging);
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const production = JSON.parse(readFileSync(process.argv[2]));
  if (process.argv[3]) {
    assert(sameBaseline(production, JSON.parse(readFileSync(process.argv[3]))),
      'Staging ledger differs from production; restore a verified staging baseline before migration');
  }
  console.log(baselineIdentity(production));
}
