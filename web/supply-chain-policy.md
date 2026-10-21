# Web dependency checks

`pnpm check:advisories` checks the full web lockfile against published advisories. High and critical findings fail the normal `pnpm check` command; lower severities are reported. There are no advisory exceptions. If one is needed, record its GHSA, affected lockfile path, reason, and expiry here before adding an explicit audit ignore.

`pnpm-workspace.yaml` also enforces the seven-day release-age and package trust policies during install. The resolver overrides move Chevrotain's pinned `lodash-es` to a patched release in the same major version, and move every `source-map-js` in the lockfile to 1.2.2, which fixes GHSA-68fv-2mgg-jv7q. The existing Trivy image scan covers built images.

`patches/env-runner@0.2.3.patch` makes Nitro's development runner open a new connection for each request it forwards to its server worker. Without it, the runner reuses pooled connections indefinitely while the worker closes them after six idle seconds, so a request sent as the worker closes its connection fails with `socket hang up` or `ECONNRESET` and Vite shows its error overlay (issue #698). `scripts/dev-runner-connections.test.mjs`, run by `pnpm test`, reproduces that race. Drop the patch once env-runner stops reusing those connections.
