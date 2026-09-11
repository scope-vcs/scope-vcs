import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

import {
  assertEffectiveRailwayDeployConfig,
  assertHealthyRailwayService,
  railwayServicesFromStatus,
  railwayServiceIsStopped,
} from "./railway-service-health.mjs";

const SOURCE_SHA = "a".repeat(40);
const expectedDeploy = {
  healthcheckPath: "/readyz",
  healthcheckTimeout: 60,
  overlapSeconds: "30",
  drainingSeconds: "30",
};

test("writer closure requires explicit nonnegative integer replica evidence", () => {
  assert.equal(railwayServiceIsStopped([{ id: "api", replicas: { running: 0, crashed: 0 } }], "api"), true);
  assert.equal(railwayServiceIsStopped([{ id: "api", replicas: { running: 1, crashed: 0 } }], "api"), false);
  for (const replicas of [undefined, null, {}, { running: 0 }, { crashed: 0 }, { running: null, crashed: 0 }, { running: "0", crashed: 0 }, { running: 0, crashed: -1 }, { running: 0.5, crashed: 0 }]) {
    assert.throws(() => railwayServiceIsStopped([{ id: "api", replicas }], "api"), /replica evidence/);
  }
  assert.throws(() => railwayServiceIsStopped([], "api"), /exactly one/);
  assert.throws(() => railwayServiceIsStopped([{ id: "api" }, { id: "api" }], "api"), /exactly one/);
});

function healthyService(id) {
  return {
    id,
    name: `name-${id}`,
    status: "SUCCESS",
    deploymentId: `deployment-${id}`,
    deploymentStopped: false,
    replicas: { configured: 2, running: 2, crashed: 0 },
    effectiveDeploy: {
      ...expectedDeploy,
      overlapSeconds: 30,
      drainingSeconds: 30,
    },
  };
}

test("rejects unavailable Railway service states", () => {
  const cases = [
    ["missing", [], /is missing/],
    ["failed probe", [{ ...healthyService("api"), status: "CRASHED" }], /is CRASHED/],
    ["stopped", [{ ...healthyService("api"), deploymentStopped: true }], /is stopped/],
    [
      "zero replicas",
      [{ ...healthyService("api"), replicas: { configured: 0, running: 0, crashed: 0 } }],
      /no configured replicas/,
    ],
    [
      "partial replicas",
      [{ ...healthyService("api"), replicas: { configured: 2, running: 1, crashed: 0 } }],
      /1\/2 running replicas/,
    ],
    [
      "crashed replicas",
      [{ ...healthyService("api"), replicas: { configured: 2, running: 2, crashed: 1 } }],
      /1 crashed replicas/,
    ],
    ["wrong deployment", [healthyService("api")], /expected previous-api/],
  ];

  for (const [name, services, expected] of cases) {
    assert.throws(
      () => assertHealthyRailwayService(services, "api", name === "wrong deployment" ? "previous-api" : ""),
      expected,
      name,
    );
  }
});

test("extracts the serving deployment and effective config from Railway status", () => {
  const services = railwayServicesFromStatus({
    environments: {
      edges: [{
        node: {
          id: "production-id",
          name: "production",
          serviceInstances: {
            edges: [{
              node: {
                serviceId: "api-id",
                serviceName: "scope-api",
                numReplicas: 1,
                latestDeployment: {
                  id: "deploying-api",
                  status: "DEPLOYING",
                  deploymentStopped: false,
                  instances: [],
                  meta: { serviceManifest: { deploy: expectedDeploy } },
                },
                activeDeployments: [{
                  id: "serving-api",
                  status: "SUCCESS",
                  deploymentStopped: false,
                  instances: [
                    { status: "RUNNING" },
                    { status: "RUNNING" },
                  ],
                  meta: {
                    serviceManifest: {
                      deploy: {
                        ...expectedDeploy,
                        multiRegionConfig: {
                          "us-east4": { numReplicas: 2 },
                        },
                      },
                    },
                  },
                }],
              },
            }],
          },
        },
      }],
    },
  }, "production-id");

  assert.equal(services[0].deploymentId, "serving-api");
  assert.equal(services[0].status, "SUCCESS");
  assert.deepEqual(services[0].replicas, {
    configured: 2,
    running: 2,
    crashed: 0,
  });
  assert.equal(services[0].effectiveDeploy.healthcheckPath, "/readyz");
});

test("effective deployment config must contain and match every transition setting", () => {
  const service = healthyService("api");
  assert.equal(
    assertEffectiveRailwayDeployConfig(service, { deploy: expectedDeploy }, "api/railway.json"),
    service,
  );

  for (const setting of Object.keys(expectedDeploy)) {
    const deploy = { ...expectedDeploy };
    delete deploy[setting];
    assert.throws(
      () => assertEffectiveRailwayDeployConfig(service, { deploy }, "api/railway.json"),
      new RegExp(`api/railway\\.json is missing deploy\\.${setting}`),
      setting,
    );

    const effectiveDeploy = { ...service.effectiveDeploy };
    delete effectiveDeploy[setting];
    assert.throws(
      () => assertEffectiveRailwayDeployConfig(
        { ...service, effectiveDeploy },
        { deploy: expectedDeploy },
      ),
      new RegExp(`effective deployment is missing deploy\\.${setting}`),
      setting,
    );
  }

  assert.throws(
    () => assertEffectiveRailwayDeployConfig(
      { ...service, effectiveDeploy: undefined },
      { deploy: expectedDeploy },
    ),
    /missing meta\.serviceManifest\.deploy/,
  );
  assert.throws(
    () => assertEffectiveRailwayDeployConfig(
      {
        ...service,
        effectiveDeploy: { ...service.effectiveDeploy, healthcheckPath: "/healthz" },
      },
      { deploy: expectedDeploy },
    ),
    /effective deploy\.healthcheckPath is "\/healthz", expected "\/readyz"/,
  );
});

test("production CLI verifies canonical receipts against the checked-in manifest and Railway status", () => {
  const root = fileURLToPath(new URL("../../", import.meta.url));
  const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url), "utf8"));
  // Model the provider contract from the actual manifest, independently of the helper's component lists.
  const configPaths = {
    cache: "cache-service/railway.json", "run-worker": "worker/railway.json",
    "git-router": "repo-router/railway.json", "media-api": "media-service/railway.json",
    api: "api/railway.json", web: "web/railway.json",
  };
  const deployments = Object.fromEntries(Object.keys(manifest.services).map(component => [component, {
    sourceSha: SOURCE_SHA, provider: "railway", evidenceId: `live-${component}`,
    ...(component === "media-worker" ? { artifactDigest: `sha256:${"b".repeat(64)}` } : {}),
  }]));
  const state = { environments: { edges: [{ node: {
    id: manifest.environments.production.environmentId, name: "production",
    serviceInstances: { edges: Object.entries(manifest.services).map(([component, service]) => ({ node: {
      serviceId: service.id, serviceName: service.name, numReplicas: 1,
      activeDeployments: [{ id: deployments[component].evidenceId, status: "SUCCESS", deploymentStopped: false,
        instances: [{ status: "RUNNING" }], meta: { serviceManifest: { deploy: {
          ...(configPaths[component] ? JSON.parse(readFileSync(`${root}${configPaths[component]}`, "utf8")).deploy : {}),
          numReplicas: 1, multiRegionConfig: { test: { numReplicas: 1 } },
        } } },
      }],
    } })) },
  } }] } };
  const run = () => spawnSync(process.execPath, [fileURLToPath(new URL("./railway-service-health.mjs", import.meta.url))], {
    cwd: root, encoding: "utf8", env: { ...process.env,
      SCOPE_RAILWAY_SERVICES_JSON: JSON.stringify(state),
      SCOPE_DEPLOYMENT_MANIFEST_JSON: JSON.stringify(manifest),
      SCOPE_PRODUCTION_DEPLOYMENTS_JSON: JSON.stringify(deployments),
    },
  });
  const success = run();
  assert.equal(success.status, 0, success.stderr);
  assert.deepEqual(JSON.parse(success.stdout).map(({ component }) => component).sort(), Object.keys(manifest.services).sort());
  delete deployments["media-worker"].artifactDigest;
  const missingDigest = run();
  assert.equal(missingDigest.status, 1);
  assert.match(missingDigest.stderr, /media-worker has no exact OCI artifact evidence/);
  deployments["media-worker"].artifactDigest = "sha256:invalid";
  const invalidDigest = run();
  assert.equal(invalidDigest.status, 1);
  assert.match(invalidDigest.stderr, /media-worker has no exact OCI artifact evidence/);
  deployments["media-worker"].artifactDigest = `sha256:${'b'.repeat(64)}`;
  delete deployments.web;
  const missingReceipt = run();
  assert.equal(missingReceipt.status, 1);
  assert.match(missingReceipt.stderr, /web has no exact Railway deployment evidence/);
});
