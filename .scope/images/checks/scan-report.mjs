import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

// Keep unfixed findings in the report. Promotion blocks fixes we can apply now.
export function actionableFindings(report) {
  if (report?.SchemaVersion !== 2 || report?.Metadata?.OS?.Family !== 'debian'
      || !Array.isArray(report.Results)
      || !report.Results.some((result) => result.Class === 'os-pkgs' && result.Type === 'debian')) {
    throw new Error('Expected a complete Trivy Debian executable-image report');
  }
  const actionable = [];
  for (const result of report.Results) {
    if (result.Vulnerabilities === undefined) continue;
    if (!Array.isArray(result.Vulnerabilities)) throw new Error('Malformed vulnerability results');
    for (const finding of result.Vulnerabilities) {
      if (typeof finding.VulnerabilityID !== 'string' || typeof finding.PkgName !== 'string'
          || !['UNKNOWN', 'LOW', 'MEDIUM', 'HIGH', 'CRITICAL'].includes(finding.Severity)
          || (finding.FixedVersion !== undefined && typeof finding.FixedVersion !== 'string')) {
        throw new Error('Malformed vulnerability finding');
      }
      if (['HIGH', 'CRITICAL'].includes(finding.Severity) && finding.FixedVersion?.trim()) {
        actionable.push({ target: result.Target, ...finding });
      }
    }
  }
  return actionable;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    if (process.argv.length !== 3) throw new Error('usage: node scan-report.mjs <Trivy JSON report>');
    const findings = actionableFindings(JSON.parse(readFileSync(process.argv[2], 'utf8')));
    for (const finding of findings) {
      console.error(`${finding.Severity} ${finding.VulnerabilityID} ${finding.PkgName}: ${finding.InstalledVersion} -> ${finding.FixedVersion}`);
    }
    console.log(`Image scan: ${findings.length} actionable HIGH/CRITICAL findings`);
    process.exitCode = findings.length ? 1 : 0;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
