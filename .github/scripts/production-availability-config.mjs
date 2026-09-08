import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { parseAvailabilityConfig } from "./release-availability-core.mjs";

const [stage, outputDirectory] = process.argv.slice(2);
if (!["backend", "web"].includes(stage) || !outputDirectory) throw new Error("usage: production-availability-config.mjs backend|web OUTPUT_DIRECTORY");
const directory = resolve(outputDirectory);
const manifest = JSON.parse(readFileSync(".github/deployment-services.json", "utf8"));
const sourceSha = process.env.SCOPE_DEPLOYMENT_SOURCE_SHA;
const config = {
  ...manifest.releaseAvailability.production,
  mode: stage === "backend" ? "maintenance" : "ordinary",
  intervalMs: 1000,
  requestTimeoutMs: 5000,
  release: {
    attemptId: `${process.env.GITHUB_RUN_ID}:${process.env.GITHUB_RUN_ATTEMPT}`,
    sourceSha,
    stage,
    deploymentsFile: resolve(directory, "deployments.json"),
  },
};
if (stage === "backend") {
  const budget = Number(process.env.SCOPE_MAINTENANCE_OUTAGE_BUDGET_MS ?? 0);
  if (!Number.isSafeInteger(budget) || budget < 0) throw new Error("Invalid maintenance outage budget");
  config.maintenance = {
    startFile: resolve(directory, "maintenance-start"),
    endFile: resolve(directory, "maintenance-end"),
    // A zero policy budget blocks a new cutover in the migration owner. Ordinary
    // activations still monitor without a window; recovery always attempts repair.
    maxDurationMs: Math.max(1, budget),
    maxFailedRequests: Math.max(1, Math.ceil(budget / 1000) * 6),
  };
}
parseAvailabilityConfig(config);
writeFileSync(resolve(directory, "config.json"), `${JSON.stringify(config, null, 2)}\n`);
