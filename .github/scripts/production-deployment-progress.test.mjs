import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import {
  latestSuccessfulDeployments,
  recordEvidenceFile,
  recordSuccessfulDeployment,
  recordSuccessfulRelease,
} from "./production-deployment-progress.mjs";

const SOURCE_SHA = "a".repeat(40);
const PREVIOUS_SHA = "b".repeat(40);

// node:test runs this file in its own process, so the GitHub credentials the
// request helper requires can be pinned once for every test below.
process.env.GITHUB_TOKEN = "test-token";
process.env.GITHUB_REPOSITORY = "scope-vcs/scope-vcs";

function response(body, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("reads the newest successful revision for every component", async () => {
  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname.endsWith("/deployments")) {
      const component = parsed.searchParams.get("environment").split("/")[1];
      return response([
        {
          id: `${component}-new`,
          sha: SOURCE_SHA,
          payload: {
            component,
            sourceSha: SOURCE_SHA,
            provider: "railway",
            evidenceId: `${component}-new-provider-id`,
          },
        },
        {
          id: `${component}-old`,
          sha: PREVIOUS_SHA,
          payload: JSON.stringify({
            component,
            sourceSha: PREVIOUS_SHA,
            provider: "railway",
            evidenceId: `${component}-old-provider-id`,
          }),
        },
      ]);
    }
    if (parsed.pathname.includes("-new/statuses")) return response([{ state: "failure" }]);
    if (parsed.pathname.includes("-old/statuses")) return response([{ state: "inactive" }, { state: "success" }]);
    throw new Error(`Unexpected request: ${url}`);
  };

  const deployments = await latestSuccessfulDeployments(fetchImpl);
  assert.equal(deployments.web.sourceSha, PREVIOUS_SHA);
  assert.equal(deployments.cache.sourceSha, PREVIOUS_SHA);
});

test("reads exact provider identity from the newest valid successful deployment", async () => {
  const fetchImpl = async (url) => {
    const parsed = new URL(url);
    if (parsed.pathname.endsWith("/deployments")) {
      const component = parsed.searchParams.get("environment").split("/")[1];
      return response([
        {
          id: `${component}-invalid`,
          sha: SOURCE_SHA,
          payload: { component, sourceSha: PREVIOUS_SHA, provider: "railway", evidenceId: "wrong" },
        },
        {
          id: `${component}-valid`,
          sha: SOURCE_SHA,
          payload: { component, sourceSha: SOURCE_SHA, provider: "railway", evidenceId: `${component}-7` },
        },
      ]);
    }
    return response([{ state: "success" }]);
  };

  const deployments = await latestSuccessfulDeployments(fetchImpl);
  assert.deepEqual(deployments.web, {
    sourceSha: SOURCE_SHA,
    provider: "railway",
    evidenceId: "web-7",
  });
});

test("records source revision and provider evidence before marking success", async () => {
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, body: JSON.parse(options.body) });
    return requests.length === 1 ? response({ id: 42 }, 201) : response({ id: 43 }, 201);
  };

  await recordSuccessfulDeployment({
    component: "web",
    sourceSha: SOURCE_SHA,
    provider: "railway",
    evidenceId: "railway-deployment-7",
    logUrl: "https://github.test/run/1",
  }, fetchImpl);

  assert.deepEqual(requests[0].body.payload, {
    component: "web",
    sourceSha: SOURCE_SHA,
    provider: "railway",
    evidenceId: "railway-deployment-7",
  });
  assert.equal(requests[0].body.environment, "production/web");
  assert.equal(requests[1].body.state, "success");
  assert.equal(requests[1].body.auto_inactive, false);
});

test("rejects unknown components before writing deployment state", async () => {
  await assert.rejects(
    recordSuccessfulDeployment({
      component: "database",
      sourceSha: SOURCE_SHA,
      provider: "railway",
      evidenceId: "deployment-7",
    }, () => { throw new Error("fetch should not run"); }),
    /Unknown deployment component/,
  );
});

test("rejects abbreviated source revisions before writing deployment state", async () => {
  await assert.rejects(
    recordSuccessfulDeployment({
      component: "web",
      sourceSha: "abc123",
      provider: "railway",
      evidenceId: "deployment-7",
    }, () => { throw new Error("fetch should not run"); }),
    /full lowercase commit SHA/,
  );
});

test("records an ordered Railway evidence stream", async () => {
  const directory = mkdtempSync(join(tmpdir(), "deployment-progress-"));
  const evidencePath = join(directory, "evidence.ndjson");
  writeFileSync(evidencePath, [
    JSON.stringify({ component: "cache", sourceSha: SOURCE_SHA, provider: "railway", evidenceId: "cache-1" }),
    JSON.stringify({ component: "run-worker", sourceSha: SOURCE_SHA, provider: "railway", evidenceId: "worker-1" }),
    "",
  ].join("\n"));
  const requests = [];
  const fetchImpl = async (url, options) => {
    requests.push({ url, body: JSON.parse(options.body) });
    const isDeployment = new URL(url).pathname.endsWith("/deployments");
    return response({ id: requests.length }, isDeployment ? 201 : 200);
  };

  try {
    const ids = await recordEvidenceFile(evidencePath, "https://github.test/run/1", fetchImpl);
    assert.deepEqual(ids, [1, 3]);
    assert.deepEqual(
      requests.filter(({ url }) => new URL(url).pathname.endsWith("/deployments"))
        .map(({ body }) => body.payload.component),
      ["cache", "run-worker"],
    );
  } finally {
    rmSync(directory, { recursive: true });
  }
});

test("aggregate success pins component evidence and keeps downtime warnings separate", async () => {
  let aggregate;
  let aggregateStatus;
  const fetchImpl = async (url, options = {}) => {
    const parsed = new URL(url);
    if (options.method === "POST") {
      const body = JSON.parse(options.body);
      if (parsed.pathname.endsWith("/deployments")) {
        aggregate = body;
        return response({ id: 99 });
      }
      aggregateStatus = body;
      return response({ id: 100 });
    }
    if (parsed.pathname.endsWith("/deployments")) {
      const component = parsed.searchParams.get("environment").split("/")[1];
      return response([{ id: component, sha: SOURCE_SHA,
        payload: { component, sourceSha: SOURCE_SHA, provider: "railway", evidenceId: `${component}-immutable-id` } }]);
    }
    return response([{ state: "success" }]);
  };
  await recordSuccessfulRelease({ sourceSha: SOURCE_SHA, components: { api: true, web: true }, warning: "Downtime exceeded 30 minutes" }, fetchImpl);
  assert.equal(aggregate.environment, "production/release");
  assert.equal(aggregate.ref, SOURCE_SHA);
  assert.equal(aggregate.payload.kind, "scope-production-release");
  assert.equal(aggregate.payload.sourceSha, SOURCE_SHA);
  assert.equal(aggregate.payload.warning, "Downtime exceeded 30 minutes");
  assert.deepEqual(Object.keys(aggregate.payload.components), ["api", "web"]);
  assert.equal(aggregate.payload.components.web.evidenceId, "web-immutable-id");
  assert.equal(aggregateStatus.state, "success");
  await assert.rejects(recordSuccessfulRelease({ sourceSha: PREVIOUS_SHA, components: ["web"] }, fetchImpl), /missing successful web evidence/);
});
