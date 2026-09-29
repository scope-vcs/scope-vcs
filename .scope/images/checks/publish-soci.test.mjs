import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, mkdirSync, readFileSync, writeFileSync, copyFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';

const executable = `sha256:${'a'.repeat(64)}`;
const artifact = `sha256:${'b'.repeat(64)}`;
const source = `sha256:${'c'.repeat(64)}`;
const manifest = () => ({
  mediaType: 'application/vnd.oci.image.index.v1+json',
  manifests: [
    { digest: executable, mediaType: 'application/vnd.oci.image.manifest.v1+json',
      platform: { os: 'linux', architecture: 'amd64' }, annotations: { 'com.amazon.soci.index-digest': artifact } },
    { digest: artifact, annotations: { 'com.amazon.soci.image-manifest-digest': executable } },
  ],
});

function publish(t, index, scanStatus = 0, sourceImage = `ghcr.io/scope/checks@${source}`) {
  const root = mkdtempSync(join(tmpdir(), 'scope-soci-scan-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const path of ['bin', '.github/scripts', '.scope/images/checks']) mkdirSync(join(root, path), { recursive: true });
  copyFileSync(new URL('../../../.github/scripts/publish-soci-image.sh', import.meta.url), join(root, '.github/scripts/publish-soci-image.sh'));
  const script = (path, body) => writeFileSync(join(root, path), `#!/bin/bash\nset -euo pipefail\n${body}\n`, { mode: 0o755 });
  writeFileSync(join(root, 'manifest.json'), JSON.stringify(index));
  script('bin/aws', `case "$2" in\n batch-get-image) cat "$FIXTURE_ROOT/manifest.json" ;;\n describe-images) echo application/vnd.amazon.soci.index.v2+json ;;\n *) exit 90 ;;\nesac`);
  for (const name of ['skopeo', 'soci']) script(`bin/${name}`, 'printf "%s\\n" "$*" >> "$FIXTURE_ROOT/copy-commands"');
  script('.scope/images/checks/scan-image.sh', 'printf "%s\\n" "$@" > "$FIXTURE_ROOT/scan-args"\nexit "$SCAN_STATUS"');
  const result = spawnSync('bash', [join(root, '.github/scripts/publish-soci-image.sh'), sourceImage,
    'registry.test/scope/checks:raw', 'registry.test/scope/checks:soci', 'scope/checks', 'raw', 'soci'], {
    cwd: root, encoding: 'utf8', env: { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, FIXTURE_ROOT: root, SCAN_STATUS: String(scanStatus) },
  });
  return { root, result };
}

test('scans the immutable executable child after copying the verified source digest', (t) => {
  const { root, result } = publish(t, manifest());
  assert.equal(result.status, 0, result.stderr);
  assert.equal(readFileSync(join(root, 'scan-args'), 'utf8'), `remote\nregistry.test/scope/checks@${executable}\nchecks-image-soci-scan.json\n`);
  const commands = readFileSync(join(root, 'copy-commands'), 'utf8');
  assert.match(commands, new RegExp(`docker://ghcr.io/scope/checks@${source}`));
  assert.match(commands, /convert --standalone --platform linux\/amd64 --min-layer-size 150000000/);
});

test('propagates a failed or unavailable scan to prevent promotion', (t) => {
  for (const status of [1, 2, 124]) assert.equal(publish(t, manifest(), status).result.status, status);
});

test('rejects mutable source tags and malformed or ambiguous executable children', (t) => {
  assert.notEqual(publish(t, manifest(), 0, 'ghcr.io/scope/checks:main').result.status, 0);
  const wrongPlatform = manifest(); wrongPlatform.manifests[0].platform.architecture = 'arm64';
  const badDigest = manifest(); badDigest.manifests[0].digest = 'invalid';
  const extraChild = manifest(); extraChild.manifests.push({ ...extraChild.manifests[0], digest: source });
  for (const index of [{}, wrongPlatform, badDigest, extraChild]) assert.notEqual(publish(t, index).result.status, 0);
});

// Read the build job's step fields and block commands, without depending on step names.
function buildSteps(workflow) {
  const jobs = workflow.split(/^jobs:\s*$/m)[1];
  const buildJob = jobs?.match(/^([ \t]+)build:\s*$/m);
  assert(buildJob, 'build job must exist');
  const build = jobs.slice(buildJob.index + buildJob[0].length)
    .split(new RegExp(`^${buildJob[1]}[\\w-]+:\\s*$`, 'm'))[0];
  const steps = [];
  const lines = build.split('\n');
  const stepsLine = lines.findIndex((line) => /^\s+steps:\s*$/.test(line));
  assert(stepsLine >= 0, 'build steps must exist');
  let step;
  let field;
  let itemIndent;
  const setField = (key, value) => {
    field = key;
    if (key === 'run') {
      step.runStyle = value;
      step.run = value === '|' || value === '>-' ? '' : value;
    } else if (key !== 'env') step[key] = value;
  };
  for (const line of lines.slice(stepsLine + 1)) {
    const item = line.match(/^(\s*)-\s+([\w-]+):\s*(.*)$/);
    if (item && (itemIndent === undefined || item[1].length === itemIndent)) {
      itemIndent = item[1].length;
      step = { env: {} };
      steps.push(step);
      setField(item[2], item[3]);
      continue;
    }
    if (!step) continue;
    const key = line.match(/^(\s+)([\w-]+):\s*(.*)$/);
    if (key && key[1].length === itemIndent + 2) {
      setField(key[2], key[3]);
    } else if (field === 'run' && (line.trim() === '' || line.length - line.trimStart().length >= itemIndent + 4)) {
      step.run += `${step.run ? '\n' : ''}${line.slice(itemIndent + 4)}`;
    } else if (field === 'env' && key && key[1].length === itemIndent + 4) {
      step.env[key[2]] = key[3];
    }
  }
  return steps.map((item) => ({
    ...item,
    run: item.runStyle === '>-' ? item.run.replace(/\n/g, ' ') : item.run,
  }));
}

function workflowPublish(t, workflow, scanStatus = 0, sociStatus = 0) {
  const root = mkdtempSync(join(tmpdir(), 'scope-soci-workflow-'));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const path of ['bin', '.github/scripts', '.scope/images/checks']) mkdirSync(join(root, path), { recursive: true });
  const script = (path, body) => writeFileSync(join(root, path), `#!/bin/bash\n${body}\n`, { mode: 0o755 });
  script('.scope/images/checks/scan-image.sh', 'printf "scan %s\\n" "$*" >> "$EVENTS"\nexit "$SCAN_STATUS"');
  script('.github/scripts/publish-soci-image.sh', 'printf "soci %s\\n" "$*" >> "$EVENTS"\nexit "$SOCI_STATUS"');
  script('bin/docker', 'printf "docker %s\\n" "$*" >> "$EVENTS"\nif [[ "$1 $2" == "image inspect" ]]; then printf "[\\"%s@%s\\"]\\n" "${3%:*}" "$PUBLISHED_DIGEST"; fi');
  const events = join(root, 'events');
  const publishedDigest = `sha256:${'b'.repeat(64)}`;
  const baseEnv = {
    ...process.env, PATH: `${root}/bin:${process.env.PATH}`, EVENTS: events,
    SCAN_STATUS: String(scanStatus), SOCI_STATUS: String(sociStatus),
    PUBLISHED_DIGEST: publishedDigest,
    GHCR_IMAGE_NAME: 'registry.test/checks', ECR_REPOSITORY: 'checks',
  };
  for (const key of ['IMAGE', 'TAGS', 'GHCR_IMAGE', 'RAW_IMAGE', 'RAW_TAG', 'SOCI_IMAGE', 'SOCI_TAG']) delete baseEnv[key];
  const context = {
    'github.event_name': 'workflow_call', 'github.ref': 'refs/heads/main',
    'github.run_attempt': '1', 'github.run_id': '101',
    'github.sha': 'd'.repeat(40), 'inputs.publish': 'true',
  };
  const steps = buildSteps(workflow);
  const relevant = steps.filter((step) => /scan-image\.sh|docker push|docker tag|publish-soci-image\.sh/.test(step.run ?? ''));
  const needed = new Set();
  const addReferences = (step) => {
    for (const value of [step.if ?? '', ...Object.values(step.env)]) {
      for (const match of value.matchAll(/\bsteps\.([\w-]+)\.outputs\.[\w-]+\b/g)) needed.add(match[1]);
    }
  };
  relevant.forEach(addReferences);
  let previousSize;
  do {
    previousSize = needed.size;
    for (const step of steps) if (needed.has(step.id)) addReferences(step);
  } while (needed.size !== previousSize);
  const outputs = new Map();
  const resolve = (value) => value.replace(/\$\{\{\s*([^}]+?)\s*\}\}/g, (_, key) => {
    const expression = key.trim();
    if (expression.startsWith('steps.')) {
      const match = expression.match(/^steps\.([\w-]+)\.outputs\.([\w-]+)$/);
      assert(match, `unsupported step expression: ${expression}`);
      const output = outputs.get(match[1])?.[match[2]];
      assert.notEqual(output, undefined, `missing ${expression}`);
      return output;
    }
    const output = expression.startsWith('env.') ? baseEnv[expression.slice(4)] : context[expression];
    assert.notEqual(output, undefined, `missing ${expression}`);
    return output;
  });
  const readOutputs = (path) => {
    if (!existsSync(path)) return {};
    const lines = readFileSync(path, 'utf8').trimEnd().split('\n');
    const result = {};
    for (let index = 0; index < lines.length; index += 1) {
      const block = lines[index].match(/^([\w-]+)<<(.+)$/);
      if (block) {
        const values = [];
        while (++index < lines.length && lines[index] !== block[2]) values.push(lines[index]);
        assert(index < lines.length, `unfinished GITHUB_OUTPUT block: ${block[1]}`);
        result[block[1]] = values.join('\n');
      } else {
        const pair = lines[index].match(/^([\w-]+)=(.*)$/);
        assert(pair, `invalid GITHUB_OUTPUT line: ${lines[index]}`);
        result[pair[1]] = pair[2];
      }
    }
    return result;
  };
  let failed = false;
  for (const [index, step] of steps.entries()) {
    if (!relevant.includes(step) && !needed.has(step.id)) continue;
    const condition = step.if?.trim().replace(/^\$\{\{\s*(.*?)\s*\}\}$/, '$1') ?? 'success()';
    const equals = condition.match(/^steps\.([\w-]+)\.outputs\.([\w-]+)\s*==\s*'true'$/);
    assert(['success()', 'always()'].includes(condition) || equals, `unexpected publish condition: ${condition}`);
    if ((failed && condition !== 'always()') || (equals && outputs.get(equals[1])?.[equals[2]] !== 'true')) continue;
    if (step.uses) {
      assert.match(step.uses, /amazon-ecr-login@/, 'unexpected external output producer');
      outputs.set(step.id, { registry: 'registry.test/ecr' });
      continue;
    }
    const outputPath = join(root, `output-${index}`);
    const env = { ...baseEnv, GITHUB_OUTPUT: outputPath };
    for (const [key, expression] of Object.entries(step.env)) env[key] = resolve(expression);
    const result = spawnSync('bash', ['-e', '-u', '-o', 'pipefail', '-c', step.run], { cwd: root, env, encoding: 'utf8' });
    assert.equal(result.error, undefined);
    if (result.status === 0 && step.id) outputs.set(step.id, readOutputs(outputPath));
    if (result.status !== 0 && step['continue-on-error'] !== 'true') failed = true;
  }
  return existsSync(events) ? readFileSync(events, 'utf8').trim().split('\n') : [];
}

test('workflow scan failures block publication and SOCI failures block mutable promotion', (t) => {
  const workflow = readFileSync(new URL('../../../.github/workflows/scope-checks-image.yml', import.meta.url), 'utf8');
  const success = workflowPublish(t, workflow);
  const scan = `scan docker registry.test/checks:sha-${'d'.repeat(40)} checks-image-scan.json`;
  assert.equal(success[0], scan);
  assert(success.some((event) => event === `docker push registry.test/checks:sha-${'d'.repeat(40)}`));
  assert(success.some((event) => event.startsWith(`soci registry.test/checks@sha256:${'b'.repeat(64)}`)));
  assert(success.some((event) => event === 'docker push registry.test/checks:main'));

  const failedLocalScan = workflowPublish(t, workflow, 1);
  assert.deepEqual(failedLocalScan, [scan]);
  const failedChildScan = workflowPublish(t, workflow, 0, 2);
  assert(failedChildScan.some((event) => event.startsWith('soci ')));
  assert(!failedChildScan.includes('docker push registry.test/checks:main'));
});
