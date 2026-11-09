import { pathToFileURL } from 'node:url';
import { RAILWAY_COMPONENTS } from './deployment-components.mjs';
import { successfulDeploymentHistory } from './production-deployment-progress.mjs';
import { readRailway } from './railway-read.mjs';
import { railwayServicesFromStatus } from './railway-service-health.mjs';

const digestPattern = /^sha256:[0-9a-f]{64}$/;
const replacementReasons = new Set(['redeploy', 'failover', 'rollback']);

function requireMatch(condition, message) {
  if (!condition) throw new Error(`Redeployed service replacement: ${message}`);
}

export function verifyRedeployedReplacement({ component, selected, receipts, history, liveDeploymentId }) {
  requireMatch(RAILWAY_COMPONENTS.includes(component), `${component || 'component'} is not a Railway service`);
  requireMatch(selected?.[component] === true, `this release does not replace ${component}`);
  const [receipt] = receipts ?? [];
  requireMatch(receipt?.provider === 'railway' && typeof receipt.evidenceId === 'string'
    && receipt.evidenceId.length > 0, `${component} has no Railway receipt`);
  requireMatch(Array.isArray(history)
    && history.every(({ createdAt }) => Number.isFinite(Date.parse(createdAt))), 'Railway deployment list is invalid');
  const imageOf = ({ evidenceId, artifactDigest }) => {
    const digest = history.find(({ id }) => id === evidenceId)?.meta?.imageDigest;
    return digestPattern.test(digest ?? '') && (!artifactDigest || artifactDigest === digest) ? digest : null;
  };
  const receipted = history.find(({ id }) => id === receipt.evidenceId);
  requireMatch(receipted?.status === 'REMOVED', 'receipted deployment has not been superseded');
  requireMatch(imageOf(receipt), 'receipted deployment has no matching image digest');
  const restorable = new Map(receipts.filter(({ provider }) => provider === 'railway').reverse()
    .map(candidate => [imageOf(candidate), candidate]).filter(([digest]) => digest));
  const receiptedAt = Date.parse(receipted.createdAt);
  const later = history.filter(({ createdAt }) => Date.parse(createdAt) > receiptedAt);
  requireMatch(later.length > 0
    && later.every(({ meta }) => replacementReasons.has(meta?.reason) && restorable.has(meta.imageDigest)),
  'a later deployment is not a redeploy or rollback of a receipted image');
  const live = later.find(({ id }) => id === liveDeploymentId);
  requireMatch(live?.status === 'SUCCESS', 'live deployment is not a successful redeploy or rollback of a receipt');
  return { ...restorable.get(live.meta.imageDigest), evidenceId: live.id };
}

export function loadRedeployedReplacement({ component, selected, receipts, status, manifest,
  railway = readRailway }) {
  const serviceId = manifest.services?.[component]?.id;
  requireMatch(typeof serviceId === 'string' && serviceId.length > 0, `${component} has no Railway service`);
  const environmentId = manifest.environments.production.environmentId;
  const live = railwayServicesFromStatus(status, environmentId).find(({ id }) => id === serviceId);
  const history = railway(['deployment', 'list', '--project', manifest.railway.projectId,
    '--service', serviceId, '--environment', environmentId, '--limit', '100', '--json']);
  return { [component]: verifyRedeployedReplacement({ component, selected, receipts, history,
    liveDeploymentId: live?.deploymentId }) };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const component = process.env.SCOPE_REPLACE_REDEPLOYED_COMPONENT;
  successfulDeploymentHistory(component, 10).then(receipts => {
    process.stdout.write(`${JSON.stringify(loadRedeployedReplacement({
      component,
      selected: JSON.parse(process.env.SCOPE_RELEASE_COMPONENTS),
      receipts: receipts.map(({ receipt }) => receipt),
      status: JSON.parse(process.env.SCOPE_RAILWAY_SERVICES_JSON),
      manifest: JSON.parse(process.env.SCOPE_DEPLOYMENT_MANIFEST_JSON),
    }))}\n`);
  }).catch(error => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
