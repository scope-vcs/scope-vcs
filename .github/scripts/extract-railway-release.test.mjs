import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtempSync, readFileSync, existsSync, mkdirSync, symlinkSync, writeFileSync, rmSync, statSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const extractor = fileURLToPath(new URL('./extract-railway-release.py', import.meta.url));
const createArchive = `
import io,json,sys,tarfile
entries=json.load(sys.stdin)
with tarfile.open(sys.argv[1],'w:gz') as archive:
 for entry in entries:
  member=tarfile.TarInfo(entry['name'])
  member.mode=entry.get('mode',0o644)
  member.type={'file':tarfile.REGTYPE,'directory':tarfile.DIRTYPE,'symlink':tarfile.SYMTYPE,'hardlink':tarfile.LNKTYPE,'fifo':tarfile.FIFOTYPE,'device':tarfile.CHRTYPE}[entry.get('type','file')]
  member.linkname=entry.get('link','')
  data=entry.get('data','runtime').encode()
  member.size=len(data) if member.isfile() else 0
  archive.addfile(member,io.BytesIO(data) if member.isfile() else None)
`;
function fixture(t, entries) {
  const root = mkdtempSync(join(tmpdir(), 'scope-release-extract-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const archive = join(root, 'release.tar.gz');
  const built = spawnSync('python3', ['-c', createArchive, archive], { input: JSON.stringify(entries), encoding: 'utf8' });
  assert.equal(built.status, 0, built.stderr);
  return { root, archive, destination: join(root, 'runtime') };
}
function extract(kind, { archive, destination }) {
  return spawnSync('python3', [extractor, kind, archive, destination], { encoding: 'utf8' });
}

test('extracts backend executables and notices as regular files with sanitized permissions', (t) => {
  const f = fixture(t, [{ name: './', type: 'directory' }, { name: './scope-vcs', mode: 0o6755, data: 'binary' }, { name: './scope-media-service', mode: 0o755, data: 'media gateway' }, { name: './LICENSE', data: 'license' }]);
  const result = extract('backend', f);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(f.destination, 'scope-vcs'), 'utf8'), 'binary');
  assert.equal(statSync(join(f.destination, 'scope-vcs')).mode & 0o7777, 0o755);
  assert.equal(readFileSync(join(f.destination, 'scope-media-service'), 'utf8'), 'media gateway');
  assert.equal(statSync(join(f.destination, 'scope-media-service')).mode & 0o7777, 0o755);
});

test('extracts the compiled web tree including hidden data', (t) => {
  const f = fixture(t, [{ name: '.output', type: 'directory' }, { name: '.output/server/index.mjs', data: 'export {}' }, { name: '.output/public/.well-known/data.json', data: '{}' }]);
  const result = extract('web', f);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(f.destination, '.output/public/.well-known/data.json'), 'utf8'), '{}');
});

for (const bad of [
  { name: '../credentials' }, { name: '/tmp/credentials' }, { name: '.output/../../credentials' },
  { name: '.output/credential-link', type: 'symlink', link: '/home/runner/.docker/config.json' },
  { name: '.output/server/index.mjs', type: 'hardlink', link: '/home/runner/.docker/config.json' },
  { name: '.output/fifo', type: 'fifo' }, { name: '.output/device', type: 'device' },
]) {
  test(`rejects ${bad.type ?? 'path'} ${bad.name} before writing any runtime files`, (t) => {
    const f = fixture(t, [{ name: '.output/server/index.mjs', data: 'valid first entry' }, bad]);
    const result = extract('web', f);
    assert.notEqual(result.status, 0);
    assert.equal(existsSync(f.destination), false);
  });
}

test('rejects publishing scripts, duplicate files and file-as-directory archives', (t) => {
  for (const [kind, entries] of [
    ['backend', [{ name: 'start.sh', data: 'steal credentials' }]],
    ['web', [{ name: '.github/scripts/publish.sh', data: 'steal credentials' }]],
    ['backend', [{ name: 'scope-vcs' }, { name: './scope-vcs' }]],
    ['web', [{ name: '.output/server/index.mjs' }, { name: '.output/server' }]],
  ]) {
    const f = fixture(t, entries);
    assert.notEqual(extract(kind, f).status, 0);
    assert.equal(existsSync(f.destination), false);
  }
});

test('rejects preexisting destination links and populated directories', (t) => {
  const f = fixture(t, [{ name: 'scope-vcs' }]);
  const credentials = join(f.root, 'credentials');
  mkdirSync(credentials);
  writeFileSync(join(credentials, 'secret'), 'do not expose');
  symlinkSync(credentials, f.destination);
  assert.notEqual(extract('backend', f).status, 0);
  assert.equal(existsSync(join(credentials, 'scope-vcs')), false);
  assert.equal(readFileSync(join(credentials, 'secret'), 'utf8'), 'do not expose');
  assert.notEqual(extract('backend', { ...f, destination: credentials }).status, 0);
});
