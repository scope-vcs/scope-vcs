# Web dependency checks

`pnpm check:advisories` checks the full web lockfile against published advisories. High and critical findings fail the normal `pnpm check` command; lower severities are reported. There are no advisory exceptions. If one is needed, record its GHSA, affected lockfile path, reason, and expiry here before adding an explicit audit ignore.

`pnpm-workspace.yaml` also enforces the seven-day release-age and package trust policies during install. The resolver overrides move Chevrotain's pinned `lodash-es` to a patched release in the same major version, and move every `source-map-js` in the lockfile to 1.2.2, which fixes GHSA-68fv-2mgg-jv7q. The existing Trivy image scan covers built images.
