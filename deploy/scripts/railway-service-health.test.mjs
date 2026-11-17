import assert from "node:assert/strict";
import test from "node:test";

import {
  assertEffectiveRailwayDeployConfig,
  assertHealthyRailwayService,
  railwayServicesFromStatus,
  railwayServiceIsStopped,
} from "./railway-service-health.mjs";

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

test("removed staging deployments are stopped without weakening health checks", () => {
  const service = { id: "worker", status: null, deploymentId: null,
    latestDeployment: null, replicas: null };
  assert.equal(railwayServiceIsStopped([service], "worker"), true);
  assert.throws(() => assertHealthyRailwayService([service], "worker"), /replica evidence/);
  for (const field of ["status", "deploymentId", "latestDeployment", "replicas"]) {
    const incomplete = { ...service };
    delete incomplete[field];
    assert.throws(() => railwayServiceIsStopped([incomplete], "worker"), /replica evidence/, field);
  }
  for (const existing of [
    { status: "SUCCESS" }, { deploymentId: "deployment" },
    { latestDeployment: { id: "queued", status: "QUEUED" } },
  ]) {
    assert.throws(() => railwayServiceIsStopped([{ ...service, ...existing }], "worker"), /replica evidence/);
  }
});

test("accepts the exact active healthy deployment", () => {
  const service = healthyService("api");
  assert.equal(
    assertHealthyRailwayService([service], "api", "deployment-api"),
    service,
  );
});

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
