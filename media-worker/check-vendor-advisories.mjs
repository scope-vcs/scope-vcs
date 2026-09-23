import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { pathToFileURL } from 'node:url';

export function checkVendorAdvisories(sbom, response, vendor) {
  assert.equal(sbom?.bomFormat, 'CycloneDX');
  const components = sbom.components?.filter((component) => component.name === 'libheif');
  assert.equal(components?.length, 1, 'SBOM must contain exactly one source-built libheif');
  assert.deepEqual(components[0], vendor.components?.[0], 'SBOM must contain the reviewed libheif source');
  assert.equal(components[0].purl, `pkg:generic/libheif@${components[0].version}`);
  assert.ok(response && typeof response === 'object' && !Array.isArray(response), 'OSV response must be an object');
  assert.ok(response.vulns === undefined || Array.isArray(response.vulns), 'OSV vulnerabilities must be an array');
  assert.ok(!response.next_page_token, 'OSV response must not be truncated');

  const vulnerabilities = response.vulns ?? [];
  for (const vulnerability of vulnerabilities) {
    assert.equal(typeof vulnerability.id, 'string', 'OSV advisory must have an ID');
    assert.ok(Array.isArray(vulnerability.affected), 'OSV advisory must describe affected releases');
  }
  const fixable = vulnerabilities.filter((vulnerability) =>
    vulnerability.affected.some((affected) =>
      affected.ranges?.some((range) => range.events?.some((event) => event.fixed))));
  return { vulnerabilities, fixable };
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    assert.equal(process.argv.length, 4, 'usage: check-vendor-advisories.mjs <completed SBOM> <OSV response>');
    const sbom = JSON.parse(readFileSync(process.argv[2], 'utf8'));
    const response = JSON.parse(readFileSync(process.argv[3], 'utf8'));
    const vendor = JSON.parse(readFileSync(new URL('./codec-vendor.cdx.json', import.meta.url), 'utf8'));
    const { vulnerabilities, fixable } = checkVendorAdvisories(sbom, response, vendor);
    for (const vulnerability of vulnerabilities) {
      console.log(`${vulnerability.id}: ${vulnerability.summary ?? 'no summary'}${fixable.includes(vulnerability) ? ' (fix available)' : ' (no published fix)'}`);
    }
    console.log(`Source-built libheif: ${vulnerabilities.length} affected OSV advisories; ${fixable.length} with a published fix`);
    if (fixable.length) process.exitCode = 1;
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
