import assert from "node:assert/strict";
import test from "node:test";

import {
  readOriginFiles,
  validateProductionOrigins,
} from "./check-production-origins.mjs";

const MANIFEST = ".github/deployment-services.json";

test("live production origins match the deployment manifest", () => {
  assert.deepEqual(validateProductionOrigins(readOriginFiles()), []);
});

test("a drifted CLI default API URL names the manifest origin it must match", () => {
  const files = readOriginFiles();
  const apiOrigin = JSON.parse(files[MANIFEST]).releaseAvailability.production.apiOrigin;
  files["cli/src/api.rs"] = files["cli/src/api.rs"].replace(
    /const DEFAULT_API_URL: &str = "[^"]+";/,
    'const DEFAULT_API_URL: &str = "https://api.drifted.test";',
  );

  assert.deepEqual(validateProductionOrigins(files), [
    `cli/src/api.rs: DEFAULT_API_URL must match ${MANIFEST} releaseAvailability.production.apiOrigin (${apiOrigin}); found https://api.drifted.test`,
  ]);
});

test("an installer URL that leaves the manifest origin fails", () => {
  const files = readOriginFiles();
  const path = "crates/scope-api-contract/src/cli_compatibility.rs";
  const cliOrigin = JSON.parse(files[MANIFEST]).environments.production.cliPublicOrigin;
  files[path] = files[path].replaceAll(cliOrigin, "https://cli.drifted.test");

  assert.deepEqual(validateProductionOrigins(files), [
    `${path}: advertised installer URL must match ${MANIFEST} environments.production.cliPublicOrigin (${cliOrigin}); found https://cli.drifted.test`,
  ]);
});

test("a dropped installer replica fails on the occurrence count", () => {
  const files = readOriginFiles();
  const path = "crates/scope-api-contract/src/cli_compatibility.rs";
  files[path] = files[path].replace(/https:\/\/[^\s"`]+\/install\.sh/, "https://example.test/get");

  assert.deepEqual(validateProductionOrigins(files), [
    `${path}: expected 2 advertised installer URL occurrence(s), found 1`,
  ]);
});

test("a manifest origin carrying a path is rejected before replicas are compared", () => {
  const files = readOriginFiles();
  const manifest = JSON.parse(files[MANIFEST]);
  manifest.environments.production.cliPublicOrigin = "https://cli.scopevcs.com/install";
  files[MANIFEST] = JSON.stringify(manifest);

  assert.deepEqual(validateProductionOrigins(files), [
    `${MANIFEST}: environments.production.cliPublicOrigin must declare the production CLI installer origin as a bare https origin`,
  ]);
});
