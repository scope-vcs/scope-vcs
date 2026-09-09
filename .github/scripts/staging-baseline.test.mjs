import assert from 'node:assert/strict';
import test from 'node:test';
import { baselineIdentity, sameBaseline } from './staging-baseline.mjs';

test('baseline identity follows applied schema independently of candidate migrations', () => {
  const current = { applied: ['m0001_initial', 'm0002_repositories'], pending: [] };
  const candidate = { ...current, pending: [{ name: 'm0003_media' }] };
  assert.equal(sameBaseline(current, candidate), true);
  assert.equal(sameBaseline(current, { applied: ['m0001_initial'] }), false);
  assert.equal(sameBaseline(current, { applied: [...current.applied, 'm0003_media'] }), false);
});

test('unknown and empty baselines cannot select a retained snapshot', () => {
  for (const applied of [undefined, [], ['m0001_initial', 'm0001_initial'], ['../snapshot']]) {
    assert.throws(() => baselineIdentity({ applied }));
  }
});
