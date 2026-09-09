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
  intervalMs: 5000,
  requestTimeoutMs: 5000,
  release: {
    attemptId: `${process.env.GITHUB_RUN_ID}:${process.env.GITHUB_RUN_ATTEMPT}`,
    sourceSha,
    stage,
    observationStartFile: resolve(directory, "observation-start"),
    deploymentsFile: resolve(directory, "deployments.json"),
  },
};
if (stage === "backend") {
  const warningSeconds = manifest.releasePolicy.maintenanceWarningSeconds;
  if (!Number.isSafeInteger(warningSeconds) || warningSeconds <= 0) throw new Error("Invalid maintenance warning threshold");
  config.maintenance = {
    startFile: resolve(directory, "maintenance-start"),
    endFile: resolve(directory, "maintenance-end"),
    warningAfterMs: warningSeconds * 1000,
  };
}
parseAvailabilityConfig(config);
writeFileSync(resolve(directory, "config.json"), `${JSON.stringify(config, null, 2)}\n`);
