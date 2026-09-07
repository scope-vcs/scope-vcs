import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, utimesSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { sourceSnapshot, saveMetadata, restoreMetadata } from './cache-metadata.mjs';

const temp = mkdtempSync(join(tmpdir(), 'scope-cache-correctness-'));
const root = join(temp, 'source');
const target = join(temp, 'target');
const archive = join(temp, 'baseline.tar');
mkdirSync(join(root, 'src'), { recursive: true });
mkdirSync(join(root, 'dep/src'), { recursive: true });
const env = { ...process.env, CARGO_TARGET_DIR: target, CARGO_INCREMENTAL: '0' };
const command = (bin, args, extra = {}) => execFileSync(bin, args, { cwd: root, env: { ...env, ...extra }, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
const write = (path, text) => writeFileSync(join(root, path), text);
function build(extra = {}) {
  return command('cargo', ['build', '--offline', '--message-format=json'], extra).trim().split('\n').map(JSON.parse)
    .find((entry) => entry.reason === 'compiler-artifact' && entry.target.name === 'cache-probe').fresh;
}
function reset() {
  command('git', ['reset', '--hard', 'HEAD']);
  for (const { path } of sourceSnapshot(root)) utimesSync(join(root, path), new Date(), new Date());
  rmSync(target, { recursive: true, force: true });
  mkdirSync(target);
  command('tar', ['-xf', archive, '-C', target]);
}
try {
  write('Cargo.toml', '[package]\nname="cache-probe"\nversion="0.1.0"\nedition="2024"\n[dependencies]\nprobe-dep={path="dep"}\n');
  write('dep/Cargo.toml', '[package]\nname="probe-dep"\nversion="0.1.0"\nedition="2024"\n');
  write('dep/src/lib.rs', 'pub fn value() -> u8 { 1 }\n');
  write('src/main.rs', 'fn main() { println!("{}:{}:{}", probe_dep::value(), env!("GENERATED_VALUE"), "A"); }\n');
  write('build.rs', 'fn main() { println!("cargo:rerun-if-changed=input.txt"); println!("cargo:rerun-if-env-changed=PROBE_ENV"); println!("cargo:rustc-env=GENERATED_VALUE={}", std::fs::read_to_string("input.txt").unwrap()); }\n');
  write('input.txt', 'one');
  command('git', ['init', '-q']);
  command('git', ['-c', 'user.name=Experiment', '-c', 'user.email=experiment@scope.test', 'add', '.']);
  assert.equal(build(), false);
  command('git', ['add', 'Cargo.lock']);
  command('git', ['-c', 'user.name=Experiment', '-c', 'user.email=experiment@scope.test', 'commit', '-qm', 'Cache fixture']);
  saveMetadata(root, target, sourceSnapshot(root));
  command('tar', ['--mtime=@0', '-cf', archive, '-C', target, '.']);
  for (const mode of ['baseline', 'timestamps-only', 'content']) {
    reset();
    restoreMetadata(root, target, mode);
    assert.equal(build(), mode === 'content', `freshness for ${mode}`);
    console.log(`PASS unchanged checkout ${mode}: ${mode === 'content' ? 'fresh' : 'rebuilt'}`);
  }
  const cases = [
    ['source-same-size', () => write('src/main.rs', readFileSync(join(root, 'src/main.rs'), 'utf8').replace('"A"', '"B"')), {}, '1:one:B'],
    ['dependency', () => write('dep/src/lib.rs', 'pub fn value() -> u8 { 2 }\n'), {}, '2:one:A'],
    ['build-input', () => write('input.txt', 'two'), {}, '1:two:A'],
    ['build-script', () => write('build.rs', 'fn main() { println!("cargo:rustc-env=GENERATED_VALUE=script"); }\n'), {}, '1:script:A'],
    ['rustflags', () => {}, { RUSTFLAGS: '-C opt-level=1' }, '1:one:A'],
    ['build-env', () => {}, { PROBE_ENV: 'changed' }, '1:one:A'],
    ['manifest', () => write('Cargo.toml', readFileSync(join(root, 'Cargo.toml'), 'utf8').replace('version="0.1.0"', 'version="0.2.0"')), {}, '1:one:A'],
  ];
  for (const [name, change, extra, expected] of cases) {
    reset(); change();
    restoreMetadata(root, target, 'content');
    assert.equal(build(extra), false, `${name} must rebuild`);
    assert.equal(command(join(target, 'debug/cache-probe'), []).trim(), expected);
    console.log(`PASS changed checkout ${name}: rebuilt and output verified`);
  }
} finally {
  rmSync(temp, { recursive: true, force: true });
}
