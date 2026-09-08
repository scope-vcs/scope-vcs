#!/usr/bin/env node

import assert from "node:assert/strict";
import { constants } from "node:fs";
import { access, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { basename, dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { chromium } from "playwright";

const DEFAULT_TRANSITION_TIMEOUT_MS = 15 * 60 * 1_000;
const DEFAULT_RECONNECT_BOUND_MS = 10_000;
const DEFAULT_ACTIVITY_TIMEOUT_MS = 60_000;

export async function verifyReleaseTransition(options) {
  validateOptions(options);
  if (await exists(options.transitionFile)) {
    throw new Error("transition marker already exists; the browser must open before activation");
  }
  if (await exists(options.activationFile)) {
    throw new Error("activation marker already exists; the browser must open before activation");
  }
  if (options.expectedActivityFile && await exists(options.expectedActivityFile)) {
    throw new Error("expected activity marker already exists; create it after the fixture update");
  }
  if (options.updateReadyFile && await exists(options.updateReadyFile)) {
    throw new Error("update-ready marker already exists; use a fresh browser evidence directory");
  }
  await Promise.all([
    rm(options.readyFile, { force: true }),
    rm(options.summaryFile, { force: true }),
  ]);

  const startedAt = new Date().toISOString();
  const pageErrors = [];
  const streamStarts = [];
  const streamEnds = [];
  const streamRequests = new Map();
  const streamPathSuffix = `/v1/repos/${encodeURIComponent(options.owner)}/${encodeURIComponent(options.repo)}/events`;
  let browser;
  let result;
  try {
    browser = await chromium.launch({ headless: true });
    const page = await browser.newPage({ viewport: { height: 900, width: 1280 } });
    page.on("pageerror", (error) => pageErrors.push(error.message));
    page.on("request", (request) => {
      if (!isEventRequest(request.url(), streamPathSuffix)) return;
      const event = { at: new Date().toISOString(), sequence: streamStarts.length + 1 };
      streamRequests.set(request, event);
      streamStarts.push(event);
    });
    const recordStreamEnd = (request, outcome) => {
      const started = streamRequests.get(request);
      if (!started || streamEnds.some(({ sequence }) => sequence === started.sequence)) return;
      streamEnds.push({ at: new Date().toISOString(), outcome, sequence: started.sequence });
    };
    page.on("requestfailed", (request) => recordStreamEnd(request, "failed"));
    page.on("requestfinished", (request) => recordStreamEnd(request, "finished"));
    const repoUrl = new URL(
      `/${encodeURIComponent(options.owner)}/${encodeURIComponent(options.repo)}`,
      `${options.baseUrl}/`,
    ).toString();
    const navigation = await page.goto(repoUrl, { timeout: 30_000, waitUntil: "domcontentloaded" });
    assert(navigation && navigation.status() < 400, "initial repository navigation failed");
    const activity = page.getByLabel("Latest repository change", { exact: true });
    const navigator = page.getByLabel("Repository file navigator", { exact: true });
    await Promise.all([activity.waitFor(), navigator.waitFor()]);
    await page.getByRole("tabpanel").waitFor();
    await page.waitForFunction(() => globalThis.__TSR_ROUTER__?.state.status === "idle");
    await waitUntil(() => streamStarts.length > 0, 30_000, "repository event stream did not connect");
    const initialActivity = await activity.innerText();
    const initialContent = await page.getByRole("tabpanel").innerText();
    const selectedTab = await page.getByRole("tab", { selected: true }).getAttribute("aria-label");
    assert(initialActivity.trim(), "initial repository activity was empty");
    assert(initialContent.trim(), "initial repository content was empty");
    assert(selectedTab, "initial repository file was not selected");
    await page.evaluate(() => {
      const state = { blanked: false, timer: 0 };
      const inspect = () => {
        const activity = document.querySelector('[aria-label="Latest repository change"]');
        const navigator = document.querySelector('[aria-label="Repository file navigator"]');
        const panel = document.querySelector('[role="tabpanel"]');
        if (!activity?.textContent?.trim() || !navigator || !panel?.textContent?.trim()) state.blanked = true;
      };
      state.timer = window.setInterval(inspect, 100);
      globalThis.__scopeReleaseWatch = state;
      inspect();
    });
    await writeJsonAtomic(options.readyFile, {
      initialActivity,
      openedAt: new Date().toISOString(),
      repo: `${options.owner}/${options.repo}`,
      streamConnections: streamStarts.length,
    });

    const activationAt = await waitForMarker(
      options.activationFile,
      options.transitionTimeoutMs,
      "timed out waiting for release activation",
    );
    const transitionAt = await waitForMarker(
      options.transitionFile,
      options.transitionTimeoutMs,
      "timed out waiting for old deployment teardown",
    );
    const reconnect = options.requireSseReconnect
      ? await waitForReconnect(
        streamStarts,
        streamEnds,
        activationAt,
        options.reconnectBoundMs,
      )
      : null;
    const blanked = await page.evaluate(() => {
      const state = globalThis.__scopeReleaseWatch;
      if (state?.timer) window.clearInterval(state.timer);
      return state?.blanked ?? true;
    });
    assert.equal(blanked, false, "repository data blanked while the deployment changed");
    assert.equal(await activity.isVisible(), true, "repository activity disappeared after reconnect");
    assert.equal(await navigator.isVisible(), true, "repository files disappeared after reconnect");

    await page.getByRole("link", { name: "Requests", exact: true }).first().click();
    await page.waitForURL((url) => url.pathname.endsWith(`/${options.owner}/${options.repo}/requests`));
    assert.equal(await activity.isVisible(), true, "repository activity disappeared during navigation");
    assert.equal(await page.getByLabel("Loading latest repository change", { exact: true }).count(), 0);
    await page.getByRole("link", { name: "Code", exact: true }).first().click();
    await page.waitForURL((url) => url.pathname.endsWith(`/${options.owner}/${options.repo}`));
    await page.getByRole("tab", { name: selectedTab, exact: true }).waitFor();
    assert.equal(await page.getByRole("tabpanel").innerText(), initialContent);
    assert.equal(await page.getByLabel("Repository file navigator", { exact: true }).isVisible(), true);

    let observedActivity = null;
    if (options.expectedActivityFile) {
      await writeJsonAtomic(options.updateReadyFile, {
        at: new Date().toISOString(),
        repo: `${options.owner}/${options.repo}`,
      });
      await waitForMarker(
        options.expectedActivityFile,
        options.activityTimeoutMs,
        "timed out waiting for fixture update marker",
      );
      const expectedActivity = (await readFile(options.expectedActivityFile, "utf8")).trim();
      assert(expectedActivity, "expected activity marker must contain text");
      await activity.getByText(expectedActivity, { exact: false }).waitFor({
        timeout: options.activityTimeoutMs,
      });
      observedActivity = await activity.innerText();
    }
    assert.deepEqual(pageErrors, []);
    result = {
      initialActivity,
      activationAt,
      observedActivity,
      pageErrors,
      passed: true,
      reconnect,
      repo: `${options.owner}/${options.repo}`,
      startedAt,
      stoppedAt: new Date().toISOString(),
      streamEnds,
      streamStarts,
      transitionAt,
    };
  } catch (error) {
    result = {
      error: safeErrorMessage(error),
      pageErrors,
      passed: false,
      repo: `${options.owner}/${options.repo}`,
      startedAt,
      stoppedAt: new Date().toISOString(),
      streamEnds,
      streamStarts,
    };
  } finally {
    await browser?.close();
  }
  await writeJsonAtomic(options.summaryFile, result);
  return result;
}

export function parseArguments(argv) {
  const values = {};
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || !value || value.startsWith("--")) {
      throw new Error("release transition arguments must be --name value pairs");
    }
    if (values[flag]) throw new Error(`duplicate argument ${flag}`);
    values[flag] = value;
  }
  const allowed = new Set([
    "--activity-timeout-ms",
    "--activation-file",
    "--base-url",
    "--expected-activity-file",
    "--ready-file",
    "--reconnect-bound-ms",
    "--repo",
    "--require-sse-reconnect",
    "--summary",
    "--transition-file",
    "--transition-timeout-ms",
    "--update-ready-file",
  ]);
  for (const flag of Object.keys(values)) {
    if (!allowed.has(flag)) throw new Error(`unknown argument ${flag}`);
  }
  const repoId = required(values, "--repo");
  const [owner, repo, extra] = repoId.split("/");
  if (!owner || !repo || extra) throw new Error("--repo must be an owner/repository pair");
  const expectedActivityFile = values["--expected-activity-file"];
  const updateReadyFile = values["--update-ready-file"];
  if (Boolean(expectedActivityFile) !== Boolean(updateReadyFile)) {
    throw new Error("--expected-activity-file and --update-ready-file must be provided together");
  }
  return {
    activationFile: required(values, "--activation-file"),
    activityTimeoutMs: positiveInteger(values["--activity-timeout-ms"], DEFAULT_ACTIVITY_TIMEOUT_MS),
    baseUrl: parseOrigin(required(values, "--base-url")),
    expectedActivityFile,
    owner,
    readyFile: required(values, "--ready-file"),
    reconnectBoundMs: positiveInteger(values["--reconnect-bound-ms"], DEFAULT_RECONNECT_BOUND_MS),
    repo,
    requireSseReconnect: booleanValue(values["--require-sse-reconnect"], true),
    summaryFile: required(values, "--summary"),
    transitionFile: required(values, "--transition-file"),
    transitionTimeoutMs: positiveInteger(
      values["--transition-timeout-ms"],
      DEFAULT_TRANSITION_TIMEOUT_MS,
    ),
    updateReadyFile,
  };
}

async function waitForReconnect(starts, ends, activationAt, boundMs) {
  let pair;
  await waitUntil(() => {
    pair = findReconnect(starts, ends, activationAt, boundMs);
    return pair !== null;
  }, boundMs * 3, `repository events did not reconnect within ${boundMs}ms`);
  return pair;
}

export function findReconnect(starts, ends, activationAt, boundMs) {
  const activationTime = Date.parse(activationAt);
  const relevantEnds = ends.filter(({ at }) => Date.parse(at) >= activationTime);
  if (relevantEnds.length === 0) return null;
  const pairs = [];
  for (const end of relevantEnds) {
    const endTime = Date.parse(end.at);
    const start = starts.find((candidate) => (
      candidate.sequence > end.sequence
    ));
    if (!start) return null;
    const reconnectMs = Math.max(0, Date.parse(start.at) - endTime);
    if (reconnectMs > boundMs) return null;
    pairs.push({
      interruptedAt: end.at,
      interruptionOutcome: end.outcome,
      reconnectedAt: start.at,
      reconnectMs,
    });
  }
  return { ...pairs.at(-1), interruptionCount: pairs.length };
}

function validateOptions(options) {
  if (Boolean(options.expectedActivityFile) !== Boolean(options.updateReadyFile)) {
    throw new Error("expected activity and update-ready files must be provided together");
  }
  const paths = [
    options.activationFile,
    options.readyFile,
    options.summaryFile,
    options.transitionFile,
  ];
  if (options.expectedActivityFile) paths.push(options.expectedActivityFile);
  if (options.updateReadyFile) paths.push(options.updateReadyFile);
  if (new Set(paths).size !== paths.length) throw new Error("browser control and evidence paths must be distinct");
  for (const path of paths) {
    if (!path?.startsWith("/") || path === "/") throw new Error("browser control and evidence paths must be absolute");
  }
}

function isEventRequest(value, suffix) {
  try {
    return new URL(value).pathname === suffix;
  } catch {
    return false;
  }
}

async function waitForMarker(path, timeoutMs, message) {
  let marker;
  await waitUntil(async () => {
    try {
      marker = await stat(path);
      return true;
    } catch (error) {
      if (error?.code === "ENOENT") return false;
      throw error;
    }
  }, timeoutMs, message);
  return marker.mtime.toISOString();
}

async function waitUntil(predicate, timeoutMs, message) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() <= deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(message);
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

function required(values, flag) {
  if (!values[flag]) throw new Error(`${flag} is required`);
  return values[flag];
}

function positiveInteger(value, fallback) {
  if (value === undefined) return fallback;
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed <= 0) throw new Error("timeouts must be positive integers");
  return parsed;
}

function booleanValue(value, fallback) {
  if (value === undefined) return fallback;
  if (value === "true") return true;
  if (value === "false") return false;
  throw new Error("--require-sse-reconnect must be true or false");
}

function parseOrigin(value) {
  const url = new URL(value);
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username || url.password || url.pathname !== "/" || url.search || url.hash
  ) {
    throw new Error("--base-url must be an HTTP origin without credentials or a path");
  }
  return url.origin;
}

function safeErrorMessage(error) {
  return error instanceof Error && error.message ? error.message : "browser transition failed";
}

async function main() {
  const result = await verifyReleaseTransition(parseArguments(process.argv.slice(2)));
  process.stdout.write(`${JSON.stringify(result)}\n`);
  if (!result.passed) process.exitCode = 1;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(safeErrorMessage(error));
    process.exitCode = 1;
  });
}
