import assert from "node:assert/strict";
import test from "node:test";

import {
  applyCreationOperations,
  assertConfigurationReady,
  configurationCanApply,
  desiredMediaState,
  generateStagingSecrets,
  normalizeLiveServiceConfig,
  normalizeLiveVariables,
  planMediaReconcile,
} from "./reconcile-media.mjs";

function manifest() {
  return {
    railway: {
      projectId: "project",
      environmentId: "production",
      regionId: "us-east4-eqdc4a",
      staging: {
        environmentId: "staging",
        environmentName: "staging",
        webDomain: "scope-web-staging.up.railway.app",
      },
    },
    services: {
      api: { id: "api", name: "scope-api" },
      media: { id: null, name: "scope-media" },
      mediaWorker: { id: null, name: "scope-media-worker" },
    },
    mediaResources: {
      bucket: { id: null, name: "scope-request-media", region: "iad" },
      production: { gatewayDomain: null, webOrigin: "https://scopevcs.com" },
      staging: { gatewayDomain: null },
      workerImage: "ghcr.io/scope-vcs/scope-media-worker",
    },
  };
}

function desired() {
  return desiredMediaState(
    manifest(),
    "staging",
    `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}`,
  );
}

function variables(bucketName = "scope-request-media") {
  return {
    DATABASE_URL: "${{scope-postgres.DATABASE_URL}}",
    SCOPE_MEDIA_BUCKET_NAME: `\${{${bucketName}.BUCKET}}`,
    SCOPE_MEDIA_BUCKET_ENDPOINT: `\${{${bucketName}.ENDPOINT}}`,
    SCOPE_MEDIA_BUCKET_ACCESS_KEY_ID: `\${{${bucketName}.ACCESS_KEY_ID}}`,
    SCOPE_MEDIA_BUCKET_SECRET_ACCESS_KEY: `\${{${bucketName}.SECRET_ACCESS_KEY}}`,
    SCOPE_MEDIA_BUCKET_REGION: `\${{${bucketName}.REGION}}`,
  };
}

function convergedState() {
  const wanted = desired();
  const state = {
    projectId: "project",
    environmentId: "staging",
    environmentName: "staging",
    buckets: [{ id: "bucket", name: "scope-request-media" }],
    services: [
      {
        id: "api",
        name: "scope-api",
        variables: {
          SCOPE_MEDIA_PUBLIC_URL: "https://${{scope-media.RAILWAY_PUBLIC_DOMAIN}}",
          SCOPE_MEDIA_GRANT_PRIVATE_KEY: "<sealed>",
        },
        variableMetadata: [{ name: "SCOPE_MEDIA_GRANT_PRIVATE_KEY", isSealed: true }],
      },
      {
        id: "gateway",
        name: "scope-media",
        variables: {
          ...variables(),
          SCOPE_MEDIA_ENCRYPTION_KEY: "<sealed>",
          SCOPE_MEDIA_GRANT_PUBLIC_KEY: "public-key",
          SCOPE_MEDIA_ALLOWED_ORIGIN: "https://scope-web-staging.up.railway.app",
        },
        variableMetadata: [{ name: "SCOPE_MEDIA_ENCRYPTION_KEY", isSealed: true }],
        config: wanted.services.gateway.config,
        domains: [{ type: "service", domain: "scope-media-staging.up.railway.app" }],
      },
      {
        id: "worker",
        name: "scope-media-worker",
        variables: { ...variables(), SCOPE_MEDIA_ENCRYPTION_KEY: "<sealed>" },
        variableMetadata: [{ name: "SCOPE_MEDIA_ENCRYPTION_KEY", isSealed: true }],
        config: wanted.services.worker.config,
        domains: [],
      },
    ],
  };
  state.projectBuckets = state.buckets.map(({ id, name }) => ({ id, name }));
  state.projectServices = state.services.map(({ id, name }) => ({ id, name }));
  return state;
}

test("plans exactly one of each missing resource and repeats without duplicates", () => {
  const empty = {
    projectId: "project",
    environmentId: "staging",
    environmentName: "staging",
    buckets: [],
    services: [{ id: "api", name: "scope-api", variables: {}, variableMetadata: [] }],
  };
  const first = planMediaReconcile(desired(), empty);
  assert.deepEqual(
    first.operations.filter(({ action }) => action.startsWith("create")),
    [
      { action: "createBucket", name: "scope-request-media", region: "iad" },
      { action: "createService", role: "gateway", name: "scope-media" },
      { action: "createService", role: "worker", name: "scope-media-worker" },
    ],
  );
  assert.deepEqual(planMediaReconcile(desired(), empty), first);
  assert.deepEqual(planMediaReconcile(desired(), convergedState()), {
    blockers: [],
    manualActions: [],
    liveDomain: "scope-media-staging.up.railway.app",
    operations: [],
  });
});

test("rejects duplicate names and requires sealed production secrets", () => {
  const state = convergedState();
  state.services.push({ ...state.services[1], id: "duplicate" });
  assert.throws(() => planMediaReconcile(desired(), state), /Multiple Railway service instances/);

  const productionDesired = desiredMediaState(
    manifest(),
    "production",
    `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}`,
  );
  const unsealed = convergedState();
  unsealed.environmentId = "production";
  unsealed.environmentName = "production";
  unsealed.services.find(({ name }) => name === "scope-media").variableMetadata[0].isSealed = false;
  assert.deepEqual(planMediaReconcile(productionDesired, unsealed).manualActions, [
    "seal scope-media.SCOPE_MEDIA_ENCRYPTION_KEY",
  ]);
});

test("attaches project resources that have no instance in the target environment", () => {
  const state = convergedState();
  state.buckets = [];
  state.services = state.services.filter(({ name }) => !name.startsWith("scope-media"));
  const result = planMediaReconcile(desired(), state);
  assert.deepEqual(result.blockers, []);
  assert.deepEqual(result.operations, [
    { action: "attachBucket", id: "bucket", name: "scope-request-media", region: "iad" },
    { action: "attachService", role: "gateway", id: "gateway", name: "scope-media" },
    { action: "attachService", role: "worker", id: "worker", name: "scope-media-worker" },
  ]);
});

test("recorded identities absent from the project block replacement creation", () => {
  const wanted = desired();
  wanted.bucket.id = "recorded-bucket";
  wanted.services.gateway.id = "recorded-gateway";
  const state = convergedState();
  state.buckets = [];
  state.projectBuckets = [];
  state.services = state.services.filter(({ name }) => name !== "scope-media");
  state.projectServices = state.projectServices.filter(({ name }) => name !== "scope-media");
  const result = planMediaReconcile(wanted, state);
  assert.deepEqual(result.blockers, [
    "manifest bucket recorded-bucket is absent from the Railway project",
    "manifest service scope-media (recorded-gateway) is absent from the Railway project",
  ]);
  assert.deepEqual(result.operations, []);
});

test("post-creation drift blocks configuration", () => {
  assert.throws(
    () => assertConfigurationReady({ blockers: ["wrong identity"], operations: [] }),
    /topology drift remains: wrong identity/,
  );
  assert.throws(
    () => assertConfigurationReady({ blockers: [], operations: [{ action: "attachService", name: "scope-media" }] }),
    /topology operations remain: attachService scope-media/,
  );
  assert.doesNotThrow(() => assertConfigurationReady({
    blockers: [],
    operations: [{ action: "setVariable", name: "SCOPE_MEDIA_ALLOWED_ORIGIN" }],
  }));
});

test("configuration waits until production secrets are sealed", () => {
  assert.equal(configurationCanApply({ manualActions: ["seal scope-media key"] }), false);
  assert.equal(configurationCanApply({ manualActions: [] }), true);
});

test("verification requires real manifest IDs and a recorded domain", () => {
  const result = planMediaReconcile(desired(), convergedState(), { requireManifestIds: true });
  assert.deepEqual(result.blockers, [
    "manifest media bucket ID is not recorded",
    "manifest media gateway domain is not recorded",
    "manifest media gateway service ID is not recorded",
    "manifest media worker service ID is not recorded",
  ]);
});

test("creation failure rolls back only resources created by this apply", async () => {
  const calls = [];
  const adapter = {
    async createBucket({ name }) { calls.push(`create:${name}`); return { id: "bucket" }; },
    async createService({ name }) {
      calls.push(`create:${name}`);
      if (name === "scope-media-worker") throw new Error("injected create failure");
      return { id: "gateway" };
    },
    async deleteBucket({ name }) { calls.push(`delete:${name}`); },
    async deleteService({ name }) { calls.push(`delete:${name}`); },
  };
  const operations = [
    { action: "createBucket", name: "scope-request-media" },
    { action: "createService", name: "scope-media" },
    { action: "createService", name: "scope-media-worker" },
  ];
  await assert.rejects(applyCreationOperations(operations, adapter), /injected create failure/);
  assert.deepEqual(calls, [
    "create:scope-request-media",
    "create:scope-media",
    "create:scope-media-worker",
    "delete:scope-media",
    "delete:scope-request-media",
  ]);
});

test("attachment failure detaches only target-environment instances", async () => {
  const calls = [];
  const adapter = {
    async attachBucket({ name, id }) { calls.push(`attach:${name}`); return { id }; },
    async attachService({ name, id }) {
      calls.push(`attach:${name}`);
      if (name === "scope-media-worker") throw new Error("injected attach failure");
      return { id };
    },
    async detachBucket({ name }) { calls.push(`detach:${name}`); },
    async detachService({ name }) { calls.push(`detach:${name}`); },
  };
  await assert.rejects(applyCreationOperations([
    { action: "attachBucket", id: "bucket", name: "scope-request-media" },
    { action: "attachService", id: "gateway", name: "scope-media" },
    { action: "attachService", id: "worker", name: "scope-media-worker" },
  ], adapter), /injected attach failure/);
  assert.deepEqual(calls, [
    "attach:scope-request-media",
    "attach:scope-media",
    "attach:scope-media-worker",
    "detach:scope-media",
    "detach:scope-request-media",
  ]);
});

test("worker source accepts only the reviewed digest-pinned GHCR image", () => {
  assert.throws(
    () => desiredMediaState(manifest(), "staging", "ghcr.io/scope-vcs/scope-media-worker:latest"),
    /pinned by sha256 digest/,
  );
});

test("fresh staging plans one key generation operation without exposing values", () => {
  const state = convergedState();
  for (const service of state.services) {
    service.variables = Object.fromEntries(
      Object.entries(service.variables).filter(([name]) => !name.includes("GRANT_") && name !== "SCOPE_MEDIA_ENCRYPTION_KEY"),
    );
    service.variableMetadata = [];
  }
  const plan = planMediaReconcile(desired(), state);
  assert.deepEqual(plan.blockers, []);
  assert.deepEqual(plan.manualActions, []);
  assert.deepEqual(plan.operations.filter(({ action }) => action === "generateStagingSecrets"), [{
    action: "generateStagingSecrets",
    services: { api: "api", gateway: "gateway", worker: "worker" },
  }]);
  assert(!JSON.stringify(plan).includes("PRIVATE KEY"));
});

test("staging keys form one signing pair and one shared encryption key", async () => {
  const values = [];
  await generateStagingSecrets(
    { api: "api", gateway: "gateway", worker: "worker" },
    async (serviceId, name, value) => values.push({ serviceId, name, value }),
  );
  assert.equal(values.length, 4);
  assert.match(values[0].value, /BEGIN PRIVATE KEY/);
  assert.match(values[1].value, /BEGIN PUBLIC KEY/);
  assert.equal(values[2].value, values[3].value);
  assert.equal(Buffer.from(values[2].value, "base64").length, 32);
});

test("normalizes Railway's nested deploy config and omitted platform restart defaults", () => {
  assert.deepEqual(normalizeLiveServiceConfig({
    deploy: {
      healthcheckPath: "/readyz",
      healthcheckTimeout: 60,
      multiRegionConfig: { region: { numReplicas: 1 } },
    },
    source: { image: `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}` },
  }), {
    healthcheckPath: "/readyz",
    healthcheckTimeout: 60,
    multiRegionConfig: { region: { numReplicas: 1 } },
    restartPolicyMaxRetries: 10,
    restartPolicyType: "ON_FAILURE",
    source: { image: `ghcr.io/scope-vcs/scope-media-worker@sha256:${"a".repeat(64)}` },
  });
});

test("unwraps raw Railway variable references without resolving them", () => {
  assert.deepEqual(normalizeLiveVariables({
    DATABASE_URL: { value: "${{scope-postgres.DATABASE_URL}}" },
    SIMPLE: "value",
  }), {
    DATABASE_URL: "${{scope-postgres.DATABASE_URL}}",
    SIMPLE: "value",
  });
});
