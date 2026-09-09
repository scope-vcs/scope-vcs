import { readFile } from "node:fs/promises";

const MAX_RESPONSE_BYTES = 1_000_000;
const APPLICATION_ERROR_TEXT = /(?:internal server error|application error|unexpected server error|service unavailable)/i;

export function parseAvailabilityConfig(value) {
  if (!isObject(value)) throw new Error("availability config must be a JSON object");
  const mode = requiredEnum(value.mode, "mode", ["ordinary", "maintenance"]);
  const webOrigin = parseOrigin(value.webOrigin, "webOrigin");
  const apiOrigin = parseOrigin(value.apiOrigin, "apiOrigin");
  const release = parseRelease(value.release);
  const fixture = parseFixture(value.fixture);
  if (value.intervalMs !== undefined && value.intervalMs !== 1_000) {
    throw new Error("intervalMs must be 1000 so release samples remain comparable");
  }
  const intervalMs = 1_000;
  const requestTimeoutMs = optionalInteger(
    value.requestTimeoutMs,
    "requestTimeoutMs",
    5_000,
    100,
    60_000,
  );
  const maintenance = mode === "maintenance"
    ? parseMaintenance(value.maintenance)
    : undefined;
  if (mode === "ordinary" && value.maintenance !== undefined) {
    throw new Error("ordinary releases cannot define a maintenance window");
  }
  return {
    apiOrigin,
    fixture,
    intervalMs,
    maintenance,
    mode,
    release,
    requestTimeoutMs,
    webOrigin,
  };
}

export function availabilityTargets(config) {
  const owner = encodeURIComponent(config.fixture.owner);
  const repo = encodeURIComponent(config.fixture.repo);
  const repoBase = `${config.apiOrigin}/v1/repos/${owner}/${repo}`;
  return [
    {
      name: "public-homepage",
      url: `${config.webOrigin}/`,
      validate: validateHomepage,
    },
    {
      name: "web-readiness",
      url: `${config.webOrigin}/readyz`,
      validate: (response) => validateReadiness(response, "web"),
    },
    {
      name: "api-readiness",
      url: `${config.apiOrigin}/readyz`,
      validate: (response) => validateReadiness(response, "api"),
    },
    {
      name: "fixture-repository",
      url: repoBase,
      validate: (response) => validateRepository(response, config.fixture),
    },
    {
      name: "fixture-file-content",
      url: `${repoBase}/files/content?path=${encodeURIComponent(config.fixture.filePath)}`,
      validate: (response) => validateFileContent(response, config.fixture),
    },
    {
      name: "fixture-request-list",
      url: `${repoBase}/requests`,
      validate: (response) => validateRequestList(response, config.fixture),
    },
  ];
}

export async function probeTarget(target, { fetchImpl = fetch, timeoutMs }) {
  const started = Date.now();
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(new Error("request timed out")), timeoutMs);
  try {
    const response = await fetchImpl(target.url, {
      // Browser navigation is covered separately. An HTML Accept header triggers
      // Clerk's development-browser handshake instead of this finite page read.
      headers: { accept: target.name === "public-homepage" ? "*/*" : "application/json" },
      redirect: "error",
      signal: controller.signal,
    });
    if (response.status !== 200) {
      await response.body?.cancel().catch(() => {});
      return failedProbe("http-status", `HTTP ${response.status}`, response.status, started);
    }
    const body = await readBoundedBody(response);
    const failure = target.validate({ body, contentType: response.headers.get("content-type") });
    return failure
      ? failedProbe(failure.kind, failure.message, response.status, started)
      : completedProbe({ ok: true, status: response.status }, started);
  } catch (error) {
    const timedOut = controller.signal.aborted;
    return failedProbe(
      timedOut ? "timeout" : "transport",
      timedOut ? "request timed out" : safeErrorMessage(error),
      undefined,
      started,
    );
  } finally {
    clearTimeout(timeout);
  }
}

export class AvailabilityEvidence {
  constructor(config, startedAt = new Date().toISOString()) {
    this.config = config;
    this.startedAt = startedAt;
    this.samples = 0;
    this.requests = 0;
    this.failures = [];
    this.targets = new Map();
    this.maintenanceWindow = null;
    this.deploymentObservations = [];
  }

  openMaintenance(at) {
    if (this.config.mode !== "maintenance" || this.maintenanceWindow) return;
    this.maintenanceWindow = { startedAt: at, endedAt: null };
  }

  closeMaintenance(at) {
    if (!this.maintenanceWindow || this.maintenanceWindow.endedAt) return;
    this.maintenanceWindow.endedAt = at;
  }

  phase(at) {
    if (!this.maintenanceWindow || at < this.maintenanceWindow.startedAt) return "serving";
    if (!this.maintenanceWindow.endedAt || at < this.maintenanceWindow.endedAt) {
      return "maintenance";
    }
    return "serving";
  }

  record(event) {
    this.requests += 1;
    const totals = this.targets.get(event.target) ?? { failures: 0, requests: 0 };
    totals.requests += 1;
    if (!event.ok) {
      totals.failures += 1;
      this.failures.push(event);
    }
    this.targets.set(event.target, totals);
    if (event.deployments) {
      this.deploymentObservations.push({ at: event.at, deployments: event.deployments });
    }
  }

  finish(stoppedAt = new Date().toISOString()) {
    const window = maintenanceWindowSummary(this.config, this.maintenanceWindow, stoppedAt);
    const failures = [...this.failures].sort(compareEventTime);
    const outsideWindowFailures = failures.filter(({ phase }) => phase !== "maintenance");
    const maintenanceFailures = failures.filter(({ phase }) => phase === "maintenance");
    const violations = [];
    if (this.config.mode === "ordinary" && failures.length > 0) {
      violations.push(`${failures.length} finite requests failed during an ordinary release`);
    }
    if (this.config.mode === "maintenance") {
      if (window && !window.endedAt) violations.push("maintenance window was not closed");
      if (window?.endedAt && Date.parse(window.endedAt) < Date.parse(window.startedAt)) {
        violations.push("maintenance window ended before it started");
      }
      if (outsideWindowFailures.length > 0) {
        violations.push(`${outsideWindowFailures.length} finite requests failed outside maintenance`);
      }
      if (maintenanceFailures.length > this.config.maintenance.maxFailedRequests) {
        violations.push(
          `${maintenanceFailures.length} maintenance failures exceeded budget ${this.config.maintenance.maxFailedRequests}`,
        );
      }
      if (window && window.durationMs > this.config.maintenance.maxDurationMs) {
        violations.push(
          `maintenance duration ${window.durationMs}ms exceeded budget ${this.config.maintenance.maxDurationMs}ms`,
        );
      }
    }
    return {
      failureCount: failures.length,
      failures,
      deploymentSnapshots: deploymentSnapshots(this.deploymentObservations),
      firstFailureAt: failures.at(0)?.at ?? null,
      lastFailureAt: failures.at(-1)?.at ?? null,
      maintenanceFailureCount: maintenanceFailures.length,
      maintenanceWindow: window,
      mode: this.config.mode,
      outsideMaintenanceFailureCount: outsideWindowFailures.length,
      passed: violations.length === 0,
      release: this.config.release,
      requestCount: this.requests,
      sampleCount: this.samples,
      startedAt: this.startedAt,
      stoppedAt,
      targets: Object.fromEntries(this.targets),
      violations,
    };
  }
}

export async function readMaintenanceMarkers(config, evidence) {
  if (config.mode !== "maintenance") return;
  const start = await markerTimestamp(config.maintenance.startFile);
  if (start) evidence.openMaintenance(start);
  const end = await markerTimestamp(config.maintenance.endFile);
  if (end) evidence.closeMaintenance(end);
}

export async function readDeploymentIds(release) {
  if (!release.deploymentsFile) return null;
  let value;
  try {
    value = JSON.parse(await readFile(release.deploymentsFile, "utf8"));
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw new Error("release deployment evidence is not valid JSON");
  }
  if (!isObject(value)) throw new Error("release deployment evidence must be an object");
  const allowed = new Set(["api", "cache", "cli", "media", "mediaWorker", "router", "web", "worker"]);
  const entries = Object.entries(value);
  if (entries.length === 0) throw new Error("release deployment evidence must not be empty");
  for (const [component, deploymentId] of entries) {
    if (!allowed.has(component) || typeof deploymentId !== "string" || !deploymentId.trim()) {
      throw new Error("release deployment evidence must map known components to deployment IDs");
    }
  }
  return Object.fromEntries(
    entries
      .map(([component, deploymentId]) => [component, deploymentId.trim()])
      .sort(([left], [right]) => left.localeCompare(right)),
  );
}

function validateHomepage({ body, contentType }) {
  if (!contentType?.toLowerCase().startsWith("text/html")) {
    return failure("content-type", "homepage did not return HTML");
  }
  if (
    APPLICATION_ERROR_TEXT.test(body) ||
    !/<title[^>]*>[^<]*scope/i.test(body) ||
    !body.includes("One repository.")
  ) {
    return failure("application", "homepage returned an application error or unexpected document");
  }
  return null;
}

function validateReadiness(response, service) {
  const parsed = parseJsonResponse(response);
  if (parsed.failure) return parsed.failure;
  if (applicationJsonFailure(parsed.value)) {
    return failure("application", `${service} readiness reported an application error`);
  }
  if (parsed.value?.status !== "ok" || parsed.value?.service !== service) {
    return failure("contract", `${service} readiness response did not report ready`);
  }
  if (
    parsed.value.checks !== undefined &&
    (!Array.isArray(parsed.value.checks) || parsed.value.checks.some((check) => check?.status !== "ok"))
  ) {
    return failure("application", `${service} readiness dependency check failed`);
  }
  return null;
}

function validateRepository(response, fixture) {
  const parsed = parseJsonResponse(response);
  if (parsed.failure) return parsed.failure;
  if (applicationJsonFailure(parsed.value)) {
    return failure("application", "repository response contained an application error");
  }
  if (
    parsed.value?.owner_handle !== fixture.owner ||
    parsed.value?.name !== fixture.repo ||
    typeof parsed.value?.id !== "string" ||
    parsed.value.id.length === 0 ||
    parsed.value.lifecycle_state !== "Ready"
  ) {
    return failure("fixture", "repository response did not match the release fixture identity");
  }
  return null;
}

function validateFileContent(response, fixture) {
  const parsed = parseJsonResponse(response);
  if (parsed.failure) return parsed.failure;
  if (applicationJsonFailure(parsed.value)) {
    return failure("application", "file response contained an application error");
  }
  const expectedPath = `/${fixture.filePath.replace(/^\/+/, "")}`;
  if (
    parsed.value?.path !== expectedPath ||
    parsed.value?.content?.kind !== "text" ||
    typeof parsed.value.content.text !== "string" ||
    !parsed.value.content.text.includes(fixture.expectedText)
  ) {
    return failure("fixture", "file response did not match the release fixture content");
  }
  return null;
}

function validateRequestList(response, fixture) {
  const parsed = parseJsonResponse(response);
  if (parsed.failure) return parsed.failure;
  if (applicationJsonFailure(parsed.value)) {
    return failure("application", "request list contained an application error");
  }
  if (
    !Array.isArray(parsed.value?.requests) ||
    parsed.value.requests.some((request) => typeof request?.id !== "string" || request.id.length === 0) ||
    fixture.expectedRequestIds.some(
      (expectedId) => !parsed.value.requests.some((request) => request.id === expectedId),
    )
  ) {
    return failure("fixture", "request list did not match the release fixture requests");
  }
  return null;
}

function parseJsonResponse({ body, contentType }) {
  if (!contentType?.toLowerCase().startsWith("application/json")) {
    return { failure: failure("content-type", "response did not return JSON") };
  }
  try {
    return { value: JSON.parse(body) };
  } catch {
    return { failure: failure("json-syntax", "response contained invalid JSON") };
  }
}

function applicationJsonFailure(value) {
  if (!isObject(value)) return false;
  if (value.success === false || value.error !== undefined && value.error !== null) return true;
  return typeof value.status === "string" && ["error", "failed", "unavailable"].includes(
    value.status.toLowerCase(),
  );
}

async function readBoundedBody(response) {
  const length = Number(response.headers.get("content-length"));
  if (Number.isFinite(length) && length > MAX_RESPONSE_BYTES) {
    await response.body?.cancel().catch(() => {});
    throw new Error("response exceeded size limit");
  }
  const body = await response.text();
  if (Buffer.byteLength(body) > MAX_RESPONSE_BYTES) throw new Error("response exceeded size limit");
  return body;
}

function failedProbe(kind, message, status, started) {
  return completedProbe({
    error: { kind, message },
    ok: false,
    ...(status === undefined ? {} : { status }),
  }, started);
}

function completedProbe(result, started) {
  const completed = Date.now();
  return {
    completedAt: new Date(completed).toISOString(),
    durationMs: completed - started,
    startedAt: new Date(started).toISOString(),
    ...result,
  };
}

function failure(kind, message) {
  return { kind, message };
}

function maintenanceWindowSummary(config, window, stoppedAt) {
  if (config.mode !== "maintenance" || !window) return null;
  const end = window.endedAt ?? stoppedAt;
  return {
    durationMs: Math.max(0, Date.parse(end) - Date.parse(window.startedAt)),
    endedAt: window.endedAt,
    maxDurationMs: config.maintenance.maxDurationMs,
    maxFailedRequests: config.maintenance.maxFailedRequests,
    startedAt: window.startedAt,
  };
}

function deploymentSnapshots(observations) {
  const snapshots = [];
  let previous = null;
  for (const observation of [...observations].sort(compareEventTime)) {
    const serialized = JSON.stringify(observation.deployments);
    if (serialized === previous) continue;
    snapshots.push(observation);
    previous = serialized;
  }
  return snapshots;
}

function compareEventTime(left, right) {
  return Date.parse(left.at) - Date.parse(right.at);
}

async function markerTimestamp(path) {
  try {
    const value = (await readFile(path, "utf8")).trim();
    if (!/^\d+$/.test(value)) throw new Error("maintenance marker must contain epoch milliseconds");
    const milliseconds = Number(value);
    if (!Number.isSafeInteger(milliseconds) || milliseconds <= 0) {
      throw new Error("maintenance marker must contain epoch milliseconds");
    }
    const timestamp = new Date(milliseconds);
    if (Number.isNaN(timestamp.valueOf())) throw new Error("maintenance marker timestamp is invalid");
    return timestamp.toISOString();
  } catch (error) {
    if (error?.code === "ENOENT") return null;
    throw error;
  }
}

function parseRelease(value) {
  if (!isObject(value)) throw new Error("release must be an object");
  const sourceSha = requiredString(value.sourceSha, "release.sourceSha");
  if (!/^[0-9a-f]{40}$/.test(sourceSha)) {
    throw new Error("release.sourceSha must be a 40-character lowercase Git SHA");
  }
  return {
    attemptId: requiredString(value.attemptId, "release.attemptId"),
    deploymentsFile: value.deploymentsFile === undefined
      ? undefined
      : requiredAbsolutePath(value.deploymentsFile, "release.deploymentsFile"),
    sourceSha,
    stage: requiredString(value.stage, "release.stage"),
  };
}

function parseFixture(value) {
  if (!isObject(value)) throw new Error("fixture must be an object");
  const filePath = requiredString(value.filePath, "fixture.filePath").replace(/^\/+/, "");
  if (!filePath) throw new Error("fixture.filePath must name a file");
  return {
    expectedText: requiredString(value.expectedText, "fixture.expectedText"),
    expectedRequestIds: stringArray(value.expectedRequestIds, "fixture.expectedRequestIds"),
    filePath,
    owner: pathSegment(value.owner, "fixture.owner"),
    repo: pathSegment(value.repo, "fixture.repo"),
  };
}

function parseMaintenance(value) {
  if (!isObject(value)) throw new Error("maintenance config is required for maintenance mode");
  return {
    endFile: requiredAbsolutePath(value.endFile, "maintenance.endFile"),
    maxDurationMs: optionalInteger(
      value.maxDurationMs,
      "maintenance.maxDurationMs",
      undefined,
      1,
      86_400_000,
    ),
    maxFailedRequests: optionalInteger(
      value.maxFailedRequests,
      "maintenance.maxFailedRequests",
      undefined,
      0,
      1_000_000,
    ),
    startFile: requiredAbsolutePath(value.startFile, "maintenance.startFile"),
  };
}

function parseOrigin(value, name) {
  const text = requiredString(value, name);
  let url;
  try {
    url = new URL(text);
  } catch {
    throw new Error(`${name} must be an HTTP origin`);
  }
  if (
    !["http:", "https:"].includes(url.protocol) ||
    url.username || url.password || url.pathname !== "/" || url.search || url.hash
  ) {
    throw new Error(`${name} must be an HTTP origin without credentials or a path`);
  }
  return url.origin;
}

function pathSegment(value, name) {
  const text = requiredString(value, name);
  if (text.includes("/")) throw new Error(`${name} must be one path segment`);
  return text;
}

function requiredAbsolutePath(value, name) {
  const text = requiredString(value, name);
  if (!text.startsWith("/") || text === "/") throw new Error(`${name} must be a specific absolute path`);
  return text;
}

function requiredString(value, name) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`${name} must be a non-empty string`);
  }
  return value.trim();
}

function requiredEnum(value, name, values) {
  if (!values.includes(value)) throw new Error(`${name} must be ${values.join(" or ")}`);
  return value;
}

function optionalInteger(value, name, fallback, minimum, maximum) {
  if (value === undefined && fallback !== undefined) return fallback;
  if (!Number.isInteger(value) || value < minimum || value > maximum) {
    throw new Error(`${name} must be an integer between ${minimum} and ${maximum}`);
  }
  return value;
}

function stringArray(value, name) {
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string" || !item.trim())) {
    throw new Error(`${name} must be an array of non-empty strings`);
  }
  const result = value.map((item) => item.trim());
  if (new Set(result).size !== result.length) throw new Error(`${name} must not contain duplicates`);
  return result;
}

function isObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function safeErrorMessage(error) {
  return error instanceof Error && error.message ? error.message : "request failed";
}
