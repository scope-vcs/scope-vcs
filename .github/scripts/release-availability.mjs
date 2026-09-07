#!/usr/bin/env node

import { constants } from "node:fs";
import { access, appendFile, readFile, rename, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  AvailabilityEvidence,
  availabilityTargets,
  parseAvailabilityConfig,
  probeTarget,
  readDeploymentIds,
  readMaintenanceMarkers,
} from "./release-availability-core.mjs";

export async function runAvailabilityProbe(paths, { fetchImpl = fetch } = {}) {
  ensureDistinctPaths(paths);
  const config = parseAvailabilityConfig(JSON.parse(await readFile(paths.config, "utf8")));
  await Promise.all([
    rm(paths.ready, { force: true }),
    rm(paths.stop, { force: true }),
    rm(paths.events, { force: true }),
    rm(paths.summary, { force: true }),
  ]);
  const evidence = new AvailabilityEvidence(config);
  const targets = availabilityTargets(config);
  let nextSample = 0;
  let writeQueue = Promise.resolve();
  const active = new Set();
  let interval;
  let controls;
  let stopping = false;
  let resolveStop;
  const stopped = new Promise((resolve) => { resolveStop = resolve; });
  const requestStop = () => {
    if (!stopping) {
      stopping = true;
      resolveStop();
    }
  };
  const onSignal = () => requestStop();
  process.once("SIGINT", onSignal);
  process.once("SIGTERM", onSignal);

  const sample = async () => {
    if (stopping) return;
    const sampleNumber = ++nextSample;
    evidence.samples += 1;
    await readMaintenanceMarkers(config, evidence);
    const deploymentsAtStart = await readDeploymentIds(config.release);
    await Promise.all(targets.map(async (target) => {
      const result = await probeTarget(target, { fetchImpl, timeoutMs: config.requestTimeoutMs });
      await readMaintenanceMarkers(config, evidence);
      const deployments = await readDeploymentIds(config.release);
      const event = {
        at: result.completedAt,
        phase: evidence.phase(result.completedAt),
        release: config.release,
        ...(deployments ? { deployments } : {}),
        ...(deploymentsAtStart ? { deploymentsAtStart } : {}),
        sample: sampleNumber,
        target: target.name,
        ...result,
      };
      evidence.record(event);
      const line = `${JSON.stringify(event)}\n`;
      writeQueue = writeQueue.then(() => appendFile(paths.events, line, { mode: 0o600 }));
    }));
  };

  const startSample = () => {
    const running = sample().catch((error) => {
      const at = new Date().toISOString();
      const event = {
        at,
        error: { kind: "probe", message: safeErrorMessage(error) },
        ok: false,
        phase: evidence.phase(at),
        release: config.release,
        sample: nextSample,
        target: "probe-runner",
      };
      evidence.record(event);
      writeQueue = writeQueue.then(() => appendFile(paths.events, `${JSON.stringify(event)}\n`, { mode: 0o600 }));
    }).finally(() => active.delete(running));
    active.add(running);
    return running;
  };

  try {
    await startSample();
    await writeQueue;
    await writeJsonAtomic(paths.ready, {
      release: config.release,
      sampledAt: new Date().toISOString(),
      targets: targets.map(({ name }) => name),
    });
    interval = setInterval(startSample, config.intervalMs);
    controls = setInterval(async () => {
      try {
        await readMaintenanceMarkers(config, evidence);
        if (await exists(paths.stop)) requestStop();
      } catch (error) {
        const at = new Date().toISOString();
        evidence.record({
          at,
          error: { kind: "control", message: safeErrorMessage(error) },
          ok: false,
          phase: evidence.phase(at),
          release: config.release,
          sample: nextSample,
          target: "probe-runner",
        });
        requestStop();
      }
    }, Math.min(250, config.intervalMs));
    await stopped;
    clearInterval(interval);
    clearInterval(controls);
    await Promise.all(active);
    await readMaintenanceMarkers(config, evidence);
    await writeQueue;
    const summary = evidence.finish();
    await writeJsonAtomic(paths.summary, summary);
    return summary;
  } catch (error) {
    const at = new Date().toISOString();
    const event = {
      at,
      error: { kind: "probe", message: safeErrorMessage(error) },
      ok: false,
      phase: evidence.phase(at),
      release: config.release,
      sample: nextSample,
      target: "probe-runner",
    };
    evidence.record(event);
    await appendFile(paths.events, `${JSON.stringify(event)}\n`, { mode: 0o600 }).catch(() => {});
    const summary = evidence.finish();
    summary.passed = false;
    summary.violations.push("availability probe could not complete");
    await writeJsonAtomic(paths.summary, summary);
    return summary;
  } finally {
    clearInterval(interval);
    clearInterval(controls);
    process.removeListener("SIGINT", onSignal);
    process.removeListener("SIGTERM", onSignal);
  }
}

function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("usage: release-availability.mjs --config FILE --events FILE --summary FILE --ready-file FILE --stop-file FILE");
    }
    if (values[flag]) throw new Error(`duplicate argument ${flag}`);
    values[flag] = value;
  }
  const allowed = new Set(["--config", "--events", "--summary", "--ready-file", "--stop-file"]);
  for (const flag of Object.keys(values)) {
    if (!allowed.has(flag)) throw new Error(`unknown argument ${flag}`);
  }
  return {
    config: requiredArgument(values, "--config"),
    events: requiredArgument(values, "--events"),
    ready: requiredArgument(values, "--ready-file"),
    stop: requiredArgument(values, "--stop-file"),
    summary: requiredArgument(values, "--summary"),
  };
}

function ensureDistinctPaths(paths) {
  const values = Object.values(paths);
  if (new Set(values).size !== values.length) throw new Error("availability control and evidence paths must be distinct");
  for (const [name, path] of Object.entries(paths)) {
    if (!path.startsWith("/") || path === "/") throw new Error(`${name} must be a specific absolute path`);
  }
}

function requiredArgument(values, name) {
  if (!values[name]) throw new Error(`${name} is required`);
  return values[name];
}

async function exists(path) {
  try {
    await access(path, constants.F_OK);
    return true;
  } catch (error) {
    if (error?.code === "ENOENT") return false;
    throw error;
  }
}

async function writeJsonAtomic(path, value) {
  const temporary = join(dirname(path), `.${basename(path)}.${process.pid}.tmp`);
  try {
    await writeFile(temporary, `${JSON.stringify(value, null, 2)}\n`, { mode: 0o600 });
    await rename(temporary, path);
  } finally {
    await rm(temporary, { force: true });
  }
}

function safeErrorMessage(error) {
  return error instanceof Error && error.message ? error.message : "availability probe failed";
}

async function main() {
  const paths = parseArguments(process.argv.slice(2));
  const summary = await runAvailabilityProbe(paths);
  process.stdout.write(`${JSON.stringify(summary)}\n`);
  if (!summary.passed) process.exitCode = 1;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(safeErrorMessage(error));
    process.exitCode = 1;
  });
}
