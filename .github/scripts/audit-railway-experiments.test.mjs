import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { auditRailwayExperiments } from "./audit-railway-experiments.mjs";

const manifest = JSON.parse(readFileSync(new URL("../deployment-services.json", import.meta.url)));
const id = "00000000-0000-0000-0000-000000000001";
const now = Date.parse("2026-09-09T18:00:00Z");
function fixture() {
  return { manifest, now, repository: "scope-vcs/scope-vcs", registry: {}, project: { id: manifest.railway.projectId, environments: {
    pageInfo: {hasNextPage: false},
    edges: ["production", "staging"].map(name => ({node: {id: manifest.environments[name].environmentId, name, createdAt: "2026-08-01T00:00:00Z"}})),
  } } };
}
function experiment(value = fixture()) {
  value.project.environments.edges.push({node: {id, name: "test-pack-index-20260909", createdAt: "2026-09-09T17:00:00Z"}});
  value.registry[id] = {owner: "adam", expiresAt: "2026-09-11T17:00:00Z"};
  return value;
}

test("actual manifest protects exactly production and staging with empty registry", () => {
  const f = fixture();
  assert.notEqual(f.project.environments.edges[0].node.id, f.project.environments.edges[1].node.id);
  assert.deepEqual(auditRailwayExperiments(f), {ok:true, protectedEnvironmentCount:2, experimentCount:0, issues:[]});

});

test("explicit future expiry is honored without an invented maximum TTL", () => {
  const f = experiment();
  assert.equal(auditRailwayExperiments(f).ok, true);
  f.registry[id].expiresAt = "2027-01-01T00:00:00.000Z";
  assert.equal(auditRailwayExperiments(f).ok, true);
});

test("expired and unregistered live experiments fail the audit", () => {
  const f = experiment();
  f.registry[id].expiresAt = "2026-09-09T18:00:00Z";
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /expired/);
  delete f.registry[id];
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /unregistered/);
});

test("owners, exact UTC expiry, valid dates and experiment names are required", () => {
  for (const expiresAt of ["2026-09-10", "2026-09-10T00:00:00+00:00", "2026-02-30T00:00:00Z", "invalid"]) {
    const f = experiment();f.registry[id].expiresAt = expiresAt;
    assert.match(auditRailwayExperiments(f).issues.join("\n"), /valid UTC/);
  }
  const f = experiment();f.registry[id].owner = " ";
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /requires an owner/);
  for (const name of ["release-proof", "test-20260909", "test-pack-20260230"]) {
    f.project.environments.edges.at(-1).node.name = name;
    assert.match(auditRailwayExperiments(f).issues.join("\n"), /must be named/);
  }
});

test("protected ID collisions, stale registrations and missing protected environments fail", () => {
  const f = experiment();
  f.registry[manifest.environments.production.environmentId] = f.registry[id];
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /must not be registered/);
  f.project.environments.edges.pop();
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /stale registration/);
  f.project.environments.edges.shift();
  assert.match(auditRailwayExperiments(f).issues.join("\n"), /Protected environment .* is missing/);
});

test("incomplete or unconfirmed inventory fails instead of hiding experiments", () => {
  const f = fixture();
  f.project.environments.pageInfo.hasNextPage = true;
  assert.throws(() => auditRailwayExperiments(f), /inventory is incomplete/);
  delete f.project.environments.pageInfo;
  assert.throws(() => auditRailwayExperiments(f), /inventory is incomplete/);
});

test("only ephemeral staging previews for open labeled pull requests bypass experiment policy", () => {
  const open = (overrides = {}) => ({number: 42, state: "open", labels: [{name: "preview"}],
    base: {ref: "main"}, head: {repo: {full_name: "scope-vcs/scope-vcs"}}, ...overrides});
  const preview = (overrides = {}, pullRequests = [open()]) => {
    const f = fixture();
    f.pullRequests = pullRequests;
    f.project.environments.edges.push({node: {id, name: "pr-42", isEphemeral: true,
      sourceEnvironment: {id: manifest.environments.staging.environmentId}, ...overrides}});
    return f;
  };
  assert.deepEqual(auditRailwayExperiments(preview()), {
    ok: true, protectedEnvironmentCount: 2, experimentCount: 0, issues: [],
  });
  for (const f of [
    preview({}, [{number: 42, state: "closed"}]),
    preview({}, [open({labels: []})]),
    preview({}, [open({base: {ref: "release"}})]),
    preview({}, [open({head: {repo: {full_name: "someone/fork"}}})]),
    preview({}, []),
    preview({isEphemeral: false}),
    preview({sourceEnvironment: {id: manifest.environments.production.environmentId}}),
    preview({sourceEnvironment: null}),
    ...["pr-042", "pr-0", "pr-42-extra", "pr-nope"].map(name => preview({name})),
  ]) {
    assert.equal(auditRailwayExperiments(f).ok, false);
    assert.match(auditRailwayExperiments(f).issues.join("\n"), /must be named test-.*unregistered/s);
  }
});
