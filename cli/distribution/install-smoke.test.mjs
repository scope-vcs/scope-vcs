import assert from 'node:assert/strict';
import { execFile, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, mkdir, readFile, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { delimiter, dirname, join, resolve } from 'node:path';
import test from 'node:test';
import { promisify } from 'node:util';

const execute = promisify(execFile);
const windows = process.platform === 'win32';
const executable = windows ? 'scope.exe' : 'scope';
const binary = resolve(process.env.SCOPE_TEST_BINARY ?? `cli/target/release/${executable}`);
const service = join(dirname(binary), windows ? 'scope-cli-service.exe' : 'scope-cli-service');
const configuration = JSON.parse(await readFile(new URL('./targets.json', import.meta.url), 'utf8'));
const platform = { linux: 'linux', darwin: 'macos', win32: 'windows' }[process.platform];
const target = configuration.targets.find(({ os, arch }) => os === platform && arch === process.arch);

async function availablePort() {
  const listener = createServer();
  listener.listen(0, '127.0.0.1');
  await once(listener, 'listening');
  const port = listener.address().port;
  await new Promise((resolve, reject) => listener.close((error) => error ? reject(error) : resolve()));
  return port;
}

async function waitForService(url, child) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    assert.equal(child.exitCode, null, 'installer service exited before becoming ready');
    try {
      const response = await fetch(`${url}/readyz`);
      if (response.ok) return;
    } catch { /* The service may still be binding its listener. */ }
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error('installer service did not become ready');
}

test('native installer installs and updates on PATH, rejects bad checksums, and preserves installed binary', { timeout: 60_000 }, async (t) => {
  assert.ok(target, `No native distribution target for ${process.platform}/${process.arch}`);
  const workspace = await mkdtemp(join(tmpdir(), 'scope installer '));
  t.after(() => rm(workspace, { recursive: true, force: true }));
  const artifacts = join(workspace, 'artifacts');
  const installDir = join(workspace, 'user bin');
  await mkdir(artifacts);
  await mkdir(installDir);
  const bytes = await readFile(binary);
  const checksum = createHash('sha256').update(bytes).digest('hex');
  // Readiness requires the complete release manifest. Only the native artifact executes.
  for (const item of configuration.targets) {
    await writeFile(join(artifacts, item.artifact), bytes);
    await writeFile(join(artifacts, `${item.artifact}.sha256`), `${checksum}  ${item.artifact}\n`);
  }
  const port = await availablePort();
  const url = `http://127.0.0.1:${port}`;
  const child = spawn(service, [], {
    env: { ...process.env, PORT: String(port), SCOPE_CLI_ARTIFACT_DIR: artifacts, SCOPE_CLI_PUBLIC_URL: url },
    stdio: ['ignore', 'ignore', 'pipe'],
  });
  let serviceError = '';
  child.stderr.on('data', (chunk) => { serviceError += chunk; });
  child.on('error', (error) => { serviceError += error.message; });
  t.after(async () => {
    if (child.exitCode === null && child.pid) {
      const exited = once(child, 'exit');
      child.kill();
      await exited;
    }
  });
  try {
    await waitForService(url, child);
  } catch (error) {
    throw new Error(`${error.message}\n${serviceError}`);
  }
  const scriptResponse = await fetch(`${url}/${windows ? 'install.ps1' : 'install.sh'}`);
  assert.ok(scriptResponse.ok);
  const script = join(workspace, windows ? 'install.ps1' : 'install.sh');
  await writeFile(script, await scriptResponse.text());
  const destination = join(installDir, executable);
  const env = { ...process.env, SCOPE_INSTALL_DIR: installDir };
  if (!windows) env.PATH = `${installDir}${delimiter}${process.env.PATH}`;
  async function install({ processOnly = false, competing = false } = {}) {
    if (!windows) return execute('sh', [script], { env, timeout: 15_000 });
    // Verify the installer changes the current PowerShell PATH, then restore the
    // runner's persisted user PATH even when installation fails.
    return execute('pwsh', ['-NoProfile', '-Command', `
      $ErrorActionPreference = 'Stop'
      $previousUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
      try {
        if ($env:SCOPE_TEST_PROCESS_ONLY -eq 'true') {
          $env:Path = "$env:SCOPE_INSTALL_DIR;$env:Path"
        }
        if ($env:SCOPE_TEST_COMPETING -eq 'true') {
          $env:Path = "$env:SCOPE_TEST_COMPETING_DIR;$env:Path"
        }
        . $env:SCOPE_TEST_INSTALL_SCRIPT
        $expectedCommand = if ($env:SCOPE_TEST_COMPETING -eq 'true') {
          Join-Path $env:SCOPE_TEST_COMPETING_DIR 'scope.exe'
        } else { Join-Path $env:SCOPE_INSTALL_DIR 'scope.exe' }
        if ((Get-Command scope).Source -ne $expectedCommand) {
          throw 'installer changed command precedence unexpectedly'
        }
        if (-not (Test-PathListContains ([Environment]::GetEnvironmentVariable('Path', 'User')) $env:SCOPE_INSTALL_DIR)) {
          throw 'install directory was not persisted in user PATH'
        }
        if ($env:SCOPE_TEST_PROCESS_ONLY -eq 'true') {
          $savedProcessPath = $env:Path
          $shell = (Get-Command pwsh).Source
          $env:Path = [Environment]::GetEnvironmentVariable('Path', 'Machine') + ';' + [Environment]::GetEnvironmentVariable('Path', 'User')
          try {
            $resolved = & $shell -NoProfile -Command '(Get-Command scope).Source'
            if ($resolved -ne (Join-Path $env:SCOPE_INSTALL_DIR 'scope.exe')) { throw 'new shell cannot resolve the installed scope' }
          } finally { $env:Path = $savedProcessPath }
        }
        & (Join-Path $env:SCOPE_INSTALL_DIR 'scope.exe') --version
        if ($LASTEXITCODE -ne 0) { throw 'installed scope failed' }
      } finally {
        [Environment]::SetEnvironmentVariable('Path', $previousUserPath, 'User')
      }
    `], { env: { ...env, SCOPE_TEST_INSTALL_SCRIPT: script, SCOPE_TEST_PROCESS_ONLY: String(processOnly), SCOPE_TEST_COMPETING: String(competing), SCOPE_TEST_COMPETING_DIR: join(workspace, 'competing bin') }, timeout: 15_000 });
  }

  await install();
  assert.deepEqual(await readFile(destination), bytes);
  assert.match((await execute(destination, ['--version'])).stdout, /^scope .+build .+protocol /);
  if (!windows) {
    const found = await execute('sh', ['-c', 'command -v scope'], { env });
    assert.equal(found.stdout.trim(), destination);
  }

  if (windows) {
    await install({ processOnly: true });
    await mkdir(join(workspace, 'competing bin'));
    await writeFile(join(workspace, 'competing bin', executable), bytes);
    await install({ competing: true });
  }

  // An existing installation must be replaced rather than skipped.
  await writeFile(destination, 'old installed version');
  await install();
  assert.deepEqual(await readFile(destination), bytes);

  await writeFile(join(artifacts, target.artifact), 'corrupted download');
  await assert.rejects(install(), (error) => {
    assert.match(`${error.stdout}\n${error.stderr}`, /checksum verification failed/);
    return true;
  });
  assert.deepEqual(await readFile(destination), bytes);
  assert.match((await execute(destination, ['--version'])).stdout, /^scope /);

  if (!windows) {
    await assert.rejects(execute('sh', [script], {
      env: { ...env, SCOPE_INSTALL_DIR: join(workspace, 'not on path') },
    }), (error) => {
      assert.match(error.stderr, /install directory is not on PATH/);
      return true;
    });
  }
});
