import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { pathToFileURL } from 'node:url';
import { setTimeout as sleep } from 'node:timers/promises';

const execute = promisify(execFile);

async function github(path, timeout) {
  const { stdout } = await execute('gh', ['api', path], {
    encoding: 'utf8', timeout, killSignal: 'SIGKILL',
  });
  return JSON.parse(stdout);
}

export async function reportPullRequestChecks({
  repository, sha, pullRequest, runId,
  request = github,
  now = () => performance.now(),
  pause = sleep,
  report = console.log,
  timeout = 140 * 60_000,
  interval = 30_000,
}) {
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository) || !/^[0-9a-f]{40}$/.test(sha)
    || !/^[1-9][0-9]*$/.test(String(pullRequest)) || !/^[1-9][0-9]*$/.test(String(runId))) {
    throw new Error('Repository, commit SHA, pull request number and run ID are required.');
  }
  const deadline = now() + timeout;
  const runPath = `repos/${repository}/actions/runs/${runId}`;
  const runUrl = `https://github.com/${repository}/actions/runs/${runId}`;
  const requireTime = () => {
    if (now() >= deadline) {
      throw new Error(`Deadline exceeded after ${timeout / 60_000} minutes waiting for ${runUrl} at ${sha}; no successful result was confirmed.`);
    }
  };
  const read = async (path) => {
    requireTime();
    try {
      return await request(path, Math.max(1, Math.min(30_000, deadline - now())));
    } catch (error) {
      report(`GitHub API request failed for ${path}: ${error.message}; retrying until the reporter deadline.`);
      return undefined;
    }
  };
  report(`Pull request #${pullRequest} tests ${sha}; waiting on ${runUrl}`);
  while (true) {
    requireTime();
    const pull = await read(`repos/${repository}/pulls/${pullRequest}`);
    requireTime();
    if (pull) {
      if (pull.state !== 'open' || pull.merged || pull.head?.sha !== sha) {
        throw new Error(`Pull request #${pullRequest} superseded ${sha}: state=${pull.state}, merged=${Boolean(pull.merged)}, head=${pull.head?.sha}; refusing to report success.`);
      }
      const run = await read(runPath);
      requireTime();
      if (run) {
        if (String(run.id) !== String(runId) || run.event !== 'pull_request' || run.head_sha !== sha) {
          throw new Error(`${runUrl} does not test pull request commit ${sha}.`);
        }
        if (run.status === 'completed') {
          report(`${runUrl}: ${run.conclusion}`);
          if (run.conclusion !== 'success') throw new Error(`Pull request run concluded ${run.conclusion}; required checks failed.`);
          return;
        }
      }
    }
    await pause(Math.min(interval, deadline - now()));
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  reportPullRequestChecks({
    repository: process.env.REPOSITORY,
    sha: process.env.SHA,
    pullRequest: process.env.PULL_REQUEST,
    runId: process.env.RUN_ID,
  }).catch(error => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
