import assert from 'node:assert/strict';
import test from 'node:test';
import { execFileSync, spawnSync } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { checkMigrationSources, migrationDigest } from './check-migration-immutability.mjs';

const name = 'm0043_retire_git_manifests.rs';
const original = 'CREATE TABLE example (state text);';
const sources = { [name]: original, 'current_schema.sql': original, 'baseline_ledger.txt': 'm0042\n' };
const locked = Object.fromEntries(Object.entries(sources).map(([path, contents]) => [path, migrationDigest(contents)]));

test('unchanged definitions and appended migrations pass', () => {
  assert.deepEqual(checkMigrationSources(sources, locked, locked), []);
  const next = 'm0044_update_example.rs';
  assert.deepEqual(checkMigrationSources({ ...sources, [next]: 'ALTER TABLE example;' },
    { ...locked, [next]: migrationDigest('ALTER TABLE example;') }, locked), []);
});

test('editing an applied definition or the frozen baseline fails', () => {
  for (const path of Object.keys(sources)) {
    assert.match(checkMigrationSources({ ...sources, [path]: `${sources[path]} changed` }, locked).join('\n'), /definition changed/);
  }
});

test('updating the checksum as well cannot conceal edits from PR checks', () => {
  const edited = { ...sources, [name]: 'changed' };
  const relocked = { ...locked, [name]: migrationDigest('changed') };
  assert.match(checkMigrationSources(edited, relocked, locked).join('\n'), /existing migration lock cannot change/);
});

test('removing migration sources or both source and checksum fails', () => {
  const remaining = { ...sources }; delete remaining[name];
  assert.match(checkMigrationSources(remaining, locked).join('\n'), /source is missing/);
  const reduced = { ...locked }; delete reduced[name];
  assert.match(checkMigrationSources(remaining, reduced, locked).join('\n'), /cannot change or disappear/);
});

test('new definitions must be locked and lock entries must identify migration definitions', () => {
  assert.match(checkMigrationSources({ ...sources, 'm0044_next.rs': 'new' }, locked).join('\n'), /new migration needs/);
  assert.match(checkMigrationSources(sources, { ...locked, 'mod.rs': migrationDigest('module') }).join('\n'), /invalid migration lock entry/);
});

test('the CLI reads the PR base lock even when both migration and current checksum change', () => {
  const root = mkdtempSync(join(tmpdir(), 'scope-migration-lock-'));
  const directory = join(root, 'crates/scope-postgres/src/migrations');
  const script = fileURLToPath(new URL('./check-migration-immutability.mjs', import.meta.url));
  try {
    mkdirSync(directory, { recursive: true });
    for (const [path, contents] of Object.entries(sources)) writeFileSync(join(directory, path), contents);
    writeFileSync(join(directory, 'sources.lock.json'), JSON.stringify(locked));
    const git = (args) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
    git(['init']); git(['add', '.']);
    git(['-c', 'user.name=Fixture', '-c', 'user.email=fixture@scope.test', 'commit', '-m', 'Freeze migration sources']);
    const base = git(['rev-parse', 'HEAD']).trim();
    const event = join(root, 'event.json');
    writeFileSync(event, JSON.stringify({ pull_request: { base: { sha: base } } }));
    const run = () => spawnSync(process.execPath, [script], {
      cwd: root, encoding: 'utf8', env: { ...process.env, GITHUB_EVENT_PATH: event },
    });
    assert.equal(run().status, 0);
    writeFileSync(join(directory, name), 'changed');
    writeFileSync(join(directory, 'sources.lock.json'), JSON.stringify({ ...locked, [name]: migrationDigest('changed') }));
    const result = run();
    assert.equal(result.status, 1);
    assert.match(result.stderr, /existing migration lock cannot change/);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});
