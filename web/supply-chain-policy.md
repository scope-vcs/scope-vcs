# Web dependency checks

`pnpm check:advisories` checks the full web lockfile against published advisories. High and critical findings fail the normal `pnpm check` command; lower severities are reported. There are no advisory exceptions. If one is needed, record its GHSA, affected lockfile path, reason, and expiry here before adding an explicit audit ignore.

`pnpm-workspace.yaml` also enforces the seven-day release-age and package trust policies during install. The resolver override moves Chevrotain's pinned `lodash-es` to a patched release in the same major version. The existing Trivy image scan covers built images.

## Release-age exceptions

- **TanStack Start 2026-09-30 release set.** CVE-2026-102989 is a critical reflected XSS in server-function responses. It is fixed in `@tanstack/start-server-core` 1.169.39 and `@tanstack/react-start` 1.168.60, which are newer than the seven-day release age. `minimumReleaseAgeExclude` lists the exact versions in that set, so later TanStack releases still wait the full seven days. Each listed version was published by TanStack's GitHub Actions with SLSA provenance. Expires 2026-10-08, after every listed version has passed the seven-day age; remove the exclusion then.
