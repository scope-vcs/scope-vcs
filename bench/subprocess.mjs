import { spawn } from 'node:child_process';
import { createReadStream } from 'node:fs';
import { performance } from 'node:perf_hooks';

export function killCommandTree(child) {
  if (!child.pid) return;
  try {
    if (process.platform === 'win32') child.kill('SIGKILL');
    else process.kill(-child.pid, 'SIGKILL');
  } catch (error) {
    if (error?.code !== 'ESRCH') child.kill('SIGKILL');
  }
}

// Own the group until close, including input failures and descendants holding pipes.
export function execute(program, args, options = {}) {
  return new Promise((resolveCommand) => {
    const child = spawn(program, args, {
      cwd: options.cwd, detached: process.platform !== 'win32',
      env: { ...process.env, ...options.env, GIT_TERMINAL_PROMPT: '0' },
      stdio: [options.stdinPath ? 'pipe' : 'ignore', 'pipe', 'pipe'],
    });
    options.activeCommands?.add(child);
    const started = options.started ?? performance.now();
    const stdout = [];
    const stderr = [];
    let stdoutBytes = 0;
    let stderrBytes = 0;
    let firstByteMs = null;
    let error = null;
    let timedOut = false;
    const fail = (cause) => {
      error ??= cause.message;
      killCommandTree(child);
    };
    child.stdout.on('data', (chunk) => {
      firstByteMs ??= performance.now() - started;
      stdoutBytes += chunk.length;
      if (options.captureStdout) stdout.push(chunk);
    });
    child.stderr.on('data', (chunk) => {
      firstByteMs ??= performance.now() - started;
      stderrBytes += chunk.length;
      if (stderrBytes <= 1024 * 1024) stderr.push(chunk);
    });
    const input = options.stdinPath ? createReadStream(options.stdinPath) : null;
    if (input) {
      input.on('error', fail);
      child.stdin.on('error', fail);
      input.pipe(child.stdin);
    }
    child.on('error', fail);
    const timer = setTimeout(() => {
      timedOut = true;
      killCommandTree(child);
    }, options.timeoutMs ?? 60_000);
    child.on('close', (code, signal) => {
      clearTimeout(timer);
      input?.destroy();
      options.activeCommands?.delete(child);
      resolveCommand({
        code, signal, error, timedOut, firstByteMs, stdoutBytes, stderrBytes,
        stdout: Buffer.concat(stdout).toString('utf8'),
        stderr: Buffer.concat(stderr).toString('utf8'),
      });
    });
  });
}
