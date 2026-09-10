import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path) => readFileSync(new URL(`../../${path}`, import.meta.url), "utf8");

test("worker release installs only the locked analyzer runtime without lifecycle scripts", () => {
  const dockerfile = read("deploy/railway/worker.Dockerfile");
  const manifests = dockerfile.indexOf("COPY dependency-analyzer/package.json dependency-analyzer/package-lock.json");
  const install = dockerfile.indexOf("npm ci --ignore-scripts --omit=dev");
  const source = dockerfile.indexOf("COPY dependency-analyzer/analyze.mjs");

  assert.match(dockerfile, /^FROM node:24\.18\.0-bookworm-slim@sha256:[0-9a-f]{64} AS analyzer-dependencies$/m);
  assert.ok(manifests >= 0 && manifests < install && install < source);
  assert.match(dockerfile, /^FROM ubuntu:24\.04$/m);
  assert.match(dockerfile, /COPY --from=analyzer-dependencies \/usr\/local\/ \/usr\/local\//);
  assert.match(dockerfile, /COPY --from=analyzer-dependencies \/app\/dependency-analyzer\/node_modules/);
  assert.match(dockerfile, /COPY dependency-analyzer\/src \.\/src/);
  assert.match(dockerfile, /COPY dependency-analyzer\/third-party-dependency-analyzer\.txt \.\//);
  assert.match(dockerfile, /SCOPE_DEPENDENCY_ANALYZER_PATH=\/app\/dependency-analyzer\/analyze\.mjs/);
  assert.match(dockerfile, /CMD \["\/app\/bin\/scope-worker"\]/);
});

test("immutable backend artifact is the sole source for the staged analyzer", () => {
  const build = read(".github/workflows/scope-api-ci.yml");
  const preparation = read(".github/workflows/prepare-release.yml");
  const image = read(".github/scripts/prepare-railway-artifact.sh");
  const extractor = read(".github/scripts/extract-railway-release.py");

  for (const file of ["package.json", "package-lock.json", "analyze.mjs"]) {
    assert.match(build, new RegExp(`dependency-analyzer/${file.replace(".", "\\.")}`));
  }
  assert.match(build, /legal\/third-party-dependency-analyzer\.txt/);
  assert.match(build, /cp -a dependency-analyzer\/src artifacts\/bin\/dependency-analyzer\/src/);
  assert.match(preparation, /cp -a artifacts\/backend\/bin\/dependency-analyzer "\$root\/dependency-analyzer"/);
  assert.match(image, /run-worker\)[\s\S]*dockerfile=deploy\/railway\/worker\.Dockerfile/);
  assert.match(image, /test ! -e "\$context_root\/dependency-analyzer\/node_modules"/);
  assert.match(extractor, /third-party-dependency-analyzer\.txt/);
  assert.match(extractor, /dependency-analyzer\/src/);
});
