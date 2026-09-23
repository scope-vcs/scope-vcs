# Dependency maintenance, September 23, 2026

This implements the [dependency audit](dependency-audit-2026-09-23.md) and the approved [implementation plan](https://web-production-4f13c9.up.railway.app/plans/scopevcs.com/1c23174cb0). The audit files retain the original inventory at `dab2cca3c3d1bce76c750e6a1dbf81a7041ab152`.

## Removed packages and their replacements

| Removed | Result |
| --- | --- |
| Pagent | Removed the staging observer, configuration, package archive, exclusive checks and notices. Invalid API responses retain structured request and validation diagnostics. The release smoke fixture now reads Scope's public repository. |
| React Doctor | Oxlint enforces Rules of Hooks and exhaustive dependencies. Resource ownership and the two source conventions remain explicit checks. The composite score and stylistic rules are gone. |
| konsistent | Babel-based checks enforce route `Route` exports and feature-page export names. |
| class-variance-authority | Typed class maps retain the existing `clsx` and Tailwind conflict handling. |
| motion | CSS handles discussion expansion, with reduced-motion support, inert collapsed content, focus handling and composer closure. |
| posthog-js and posthog-rs | Scope keeps PostHog and its existing event/privacy rules. Small transports send events through fetch/sendBeacon and Reqwest/Tokio. |
| dependency-cruiser | One Babel syntax pass feeds enhanced-resolve. JSONC configuration and snapshot containment remain checked. |

TypeScript 7.0.2 checks the web application and emits its tests. Source analysis no longer imports the TypeScript compiler API. Neither project retains TypeScript 5 or 6, an old-compiler bridge, or a second resolver path.

The removed observer boundary belonged only to Pagent. Resource boundaries still run. Hooks and convention checks include passing and failing fixtures. `pnpm check:advisories` checks the entire web lockfile and fails on high or critical advisories. It currently has no exceptions. Install-time release-age and provenance policies remain enabled; see [the web supply-chain policy](../web/supply-chain-policy.md).

## Analytics delivery policy

The browser queues at most 64 waiting events; the server queues 128. Both reject events larger than 16 KiB, use two-second request deadlines, and retry transient failures at most twice. Events receive an ID and capture timestamp before serialization, so retries retain their identity. Queues are memory-only and can lose events on saturation, network failure or shutdown.

Browser logout clears pending events and aborts the current request. DNT disables capture and identity storage. Page exit attempts best-effort beacon delivery for the active payload and queued events, retaining their original IDs and timestamps. The server allows two seconds for shutdown flushing and then drops remaining work. Neither adapter adds `$set`, `$set_once` or `$unset` payloads or automatic capture. Existing identity transitions, property filtering and person-profile policy remain in their original owners.

## Versions and deliberate holds

Rust moves to 1.98.1, Reqwest to 0.13.5, SeaORM to 2.0.3 and SQLx to 0.9.0. HMAC, SHA-1/SHA-2 and ChaCha20-Poly1305 move together. Ciphertext formats and Git object IDs remain unchanged. SeaORM's mock feature is test-only. Its migration dependency still enables Chrono through upstream defaults; this change does not claim to remove Chrono from the cache service.

The AWS clients no longer select the legacy HTTP/TLS stack. The workspace lock loses Hyper 0.14, rustls 0.21 and Reqwest 0.12. Rustls, event-listener, anyhow, lru and the CLI browser launcher receive the reviewed fixes. The RSA advisory from the audit remains use-specific: Scope verifies signatures rather than using RSA decryption.

Git 2.55.0 has one version/source-checksum owner in [dev/tool-versions.json](../dev/tool-versions.json). Server images build that source; the CLI checks the host version. Request merges pass the recorded base directly to `merge-tree --merge-base`, removing three synthetic commits. Workflow blob reads use one bounded `cat-file --batch` process per operation. No repository object-format or ref-storage migration is included.

The web uses React 19.3, Vite 8.3, Playwright 1.63 and the corresponding reviewed UI packages. The newest eligible Clerk, Start, Router and Lucide versions are 1.5.15, 1.168.54, 1.170.36 and 1.46.0. Newer releases failed the seven-day age policy at implementation time.

Pierre stays at 1.2.11. Version 1.2.12 introduces `@pierre/theme` 1.1.0 without the provenance of its predecessor; 1.4.3 introduces the same issue through theme 2.0.0. Both fail the existing trust-downgrade check. The policy was not relaxed. The `lodash-es` override stays within major 4 and replaces Chevrotain's vulnerable exact pin.

The custom TanStack Start configuration now includes the framework's server-function CSRF middleware. HTTP checks reject cross-site requests and requests without origin metadata, while hydrated browser calls continue to work.

Node images use 24.21.0 and primary database tests use PostgreSQL 18.6. Python recovery dependencies have a hashed transitive lock. Cross, image digests, Railway CLI and external workflow actions have reviewed pins. The media image uses FFmpeg 7.1.5 and source-built libheif 1.23.5, retaining the verified source archive and build recipe. Trivy checks packaged components; a separate OSV query checks the pinned libheif PURL because Trivy does not detect that source build. OSV reports two unresolved OSS-Fuzz records (OSV-2020-2308 and OSV-2023-1129) without a published fix. The workflow retains their raw response and fails when an affected advisory has a published fix. This is not a claim that the media image has no known advisories.

## Measured changes

These are registry package entries per lockfile, including optional/platform entries. The same dependency can appear in more than one lockfile.

| Lockfile | Before | After | Change |
| --- | ---: | ---: | ---: |
| Workspace Cargo | 483 | 465 | -18 |
| CLI Cargo | 276 | 281 | +5 |
| Web pnpm | 759 | 538 | -221 |
| Analyzer npm | 44 | 8 | -36 |
| Total | 1,562 | 1,292 | -270 |

On this Linux host, normal/build dependency graphs changed from 362 to 340 package versions for the API, 332 to 300 for the worker, and 294 to 308 for the cache service. These graphs overlap and must not be summed. The cache graph grows with the ORM and HTTP migrations. Lockfile reduction does not imply every binary gets smaller.

Two runs on the same integrated web source measured TypeScript 7 at 9.92 and 8.76 seconds, versus TypeScript 6.0.3 at 40.08 and 53.02 seconds. Peak resident memory was about 695 to 744 MiB versus 862 to 868 MiB. Other builds were running on the host, so these are indicative measurements, not a controlled performance guarantee. TypeScript 6 was used only from a temporary comparison checkout.

The old and new analyzers produced identical results on 466 copied web files: 1,516 dependency edges and zero gaps. Twenty-four fixtures cover imports, package conditions, aliases, inherited root directories, module suffixes, configuration errors, containment and output bounds. After the review fixes, the preceding resolver and final resolver also agree on 468 current web files: 1,523 edges and zero gaps. A disposable 64-blob Git benchmark over ten runs measured 401.08 ms median for separate processes and 10.65 ms for one batch.

## Validation

- Combined web typecheck, 528 unit tests, Hooks/resource/convention checks, advisory check and production build pass.
- Analyzer tests and the old/new graph comparison pass.
- A local Reqwest probe rejects an untrusted TLS certificate, accepts an explicitly trusted one and verifies environment HTTP proxy forwarding.
- License declarations and notices were regenerated from checksum-verified archives and published source revisions.
- The 268 CLI tests, repository policy checks and vendor-advisory gate fixtures pass. An actual CLI invocation rejects Git 2.54 before initialization writes anything.
- The SeaORM 2 migration harness now exercises the same outer transaction as production: ledger installation and failed transforms must roll back together.
- Rust workspace tests, API feature tests and workspace Clippy pass. PostgreSQL 18 cluster, cutover and runtime-role checks pass in disposable containers.
- The integrated media image passes nine Rust tests and all codec assertions for 12 valid and three invalid fixtures. Explicit encoder color metadata preserves the intended output under FFmpeg 7.1.5. Capping glibc allocator arenas keeps virtual memory below the existing 1,536 MiB process limit; the large-video probe fell from about 1,534 MiB to 1,061 MiB without changing encoder settings.
- The media image scan reports 596 OS advisory entries, including 208 high or critical entries, with no available fixes for those high or critical entries. The separate libheif gate reports the two unresolved records described above. Both configured gates pass.
- All 58 browser smoke tests and the two-actor CLI contribution flow pass against the upgraded local stack. Navigation reuse tests wait for initial live reconciliation before measuring requests.
- AutoReview completed at P1 with no findings for the implementation and subsequent integration fixes. The PR records hosted CI and platform results.
