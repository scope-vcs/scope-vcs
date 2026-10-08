# Web dependency checks

`pnpm check:advisories` checks the full web lockfile against published advisories. High and critical findings fail the normal `pnpm check` command; lower severities are reported. There are no advisory exceptions. If one is needed, record its GHSA, affected lockfile path, reason, and expiry here before adding an explicit audit ignore.

`pnpm-workspace.yaml` also enforces the seven-day release-age and package trust policies during install. The resolver overrides move Chevrotain's pinned `lodash-es` to a patched release in the same major version, and move every `source-map-js` in the lockfile to 1.2.2, which fixes GHSA-68fv-2mgg-jv7q. The existing Trivy image scan covers built images.

## Release-age exceptions

- **`@tanstack/charts` 1.0.0.** The repository settings visibility timeline (#542) uses the 1.0 release of TanStack Charts, published on 2026-10-03 with an npm SLSA provenance attestation. All of its peer dependencies are optional. `minimumReleaseAgeExclude` lists only `@tanstack/charts@1.0.0`, so later releases still wait the full seven days. Approved by Adam Blumoff (@adamblumoff), who merged #542 with this exception described. Expires 2026-10-10 15:09 UTC; remove the exclusion then.
