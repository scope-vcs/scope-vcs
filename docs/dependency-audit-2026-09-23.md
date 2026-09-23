# Dependency audit, September 23, 2026

I recommend keeping 98 of the 106 distinct direct packages, replacing six narrow integrations with code we own, and removing two tools or integrations after addressing the behavior they provide. The largest practical reduction is in development tooling. The best immediate backend reduction is unused AWS transport features. Rewriting Git, cryptography, authentication, database drivers, Markdown parsers or media codecs would make Scope harder to maintain.

This review covers commit `dab2cca3c3d1bce76c750e6a1dbf81a7041ab152`. It includes the workspace and standalone CLI manifests, both JavaScript projects, Python requirements, Dockerfiles, workflow actions, downloaded tools, service integrations and copied UI sources. Application code, manifests, lockfiles and deployed services were not changed.

This is the pre-implementation inventory. Subsequent changes and measurements are recorded in the [implementation notes](dependency-maintenance-2026-09-23.md).

The detailed results are in these files:

- [Every direct package, with a keep/delete/rewrite decision](dependency-audit-2026-09-23/direct-dependencies.md).
- [External tools, platforms and services](dependency-audit-2026-09-23/external-dependencies.md).
- [Direct package CSV](dependency-audit-2026-09-23/direct-dependencies.csv), including declarations, resolved versions, use sites and registry links.
- [All 1,562 source-lockfile package entries](dependency-audit-2026-09-23/locked-dependencies.csv), including parent relationships and recommendations through those parents.
- [Advisory matches](dependency-audit-2026-09-23/advisory-matches.csv) and [research observations and experiment results](dependency-audit-2026-09-23/research.json).

## What I would change first

| Priority | Recommendation | Reason and expected result |
| --- | --- | --- |
| First | Update Rust 1.98.0 to 1.98.1 everywhere it builds release binaries | This fixes a compiler miscompilation involving trait-object vtables. It takes priority over cosmetic dependency cleanup. |
| First | Refresh affected Rust transitive dependencies and CLI patches | Both Rust locks contain rustls 0.23.41. The CLI also has older anyhow, lru and webbrowser versions with published advisories. See the assessment below. |
| First | Remove the AWS SDKs' legacy TLS feature selection | A temporary-manifest experiment removed seven package versions from both API and worker normal/build graphs, while retaining the modern HTTPS client. |
| First | Update DOMPurify and the dated Nitro beta, then refresh affected web transitives | DOMPurify 3.4.15 adds XML hardening. Nitro's current March beta predates two route-rule fixes, though the affected configuration was not found here. |
| Next | Pin Git 2.55.0 in server/check images and declare the CLI requirement | Local Git is already 2.55.0. Production Dockerfiles currently select distribution Git without a version contract. |
| Next | Replace synthetic merge-base commits with `merge-tree --merge-base` | Removes three preparation subprocesses and three temporary commit objects per merge attempt. Preserve Scope's explicit request-base rule. |
| Next | Replace `konsistent`, `motion`, and `class-variance-authority` in their limited uses | These replace a two-rule checker, one animation and a few style maps. Gross reduction is ten web lock snapshots, before any new dependency costs. |
| Next | Remove `pagent` if its staging investigation feature is not worth retaining | One staging-only event, one vendored package, zero transitive packages. Keep structured invalid-response diagnostics. This removes a behavior, not dead code. |
| Planned | Replace the mandatory `react-doctor` installation with explicit checks | Largest tooling opportunity, 228 exclusive lock snapshots. Preserve Hooks linting, architecture checks and required supply-chain coverage before removing the broad CLI. |
| Planned | Replace event-only PostHog SDK adapters | Existing Scope modules already own event policy. Keep the PostHog service; implement bounded transport behind existing interfaces. The web SDK has ten exclusive snapshots. |
| Later | Consider replacing `dependency-cruiser` with a narrow TypeScript resolver | Could reduce the standalone analyzer from 44 installed lock entries to TypeScript and local code. It does not remove Node or TypeScript. Module resolution equivalence is the hard part. |

The Rust patch fixes actual incorrect code generation, not just diagnostics. Update `rust-toolchain.toml`, checks/media builder images, pinned digests and any version assertions together. I have not established whether Scope triggers the compiler bug. [Rust 1.98.1 announcement](https://blog.rust-lang.org/2026/09/03/Rust-1.98.1/).

The direct-package decisions are 58 Rust packages, 44 distinct npm packages and four Python packages. TypeScript appears in both npm projects. Repeated declarations across Rust crates count once. The four main lockfiles contain 483 workspace Rust entries, 276 CLI Rust entries, 759 web package entries and 44 analyzer entries. These totals overlap across lockfiles and include optional/platform packages. They are not production binary or browser bundle sizes.

## Where fewer dependencies pays off

The AWS selection is the strongest finding. `Cargo.toml` enables both `default-https-client` and the SDK's older `rustls` feature for S3. Lambda enables both through defaults. Cargo's feature graph shows that the older feature selects `aws-smithy-runtime/tls-rustls`, `hyper-014` and `legacy-rustls-ring`. It brings in a second HTTP/TLS generation.

In a temporary copy of the manifests, I removed S3's `rustls` feature and selected `default-https-client` plus `rt-tokio` explicitly for Lambda. The API and worker graphs both lost `h2 0.3.27`, `hyper 0.14.32`, `hyper-rustls 0.24.2`, `rustls 0.21.12`, `rustls-webpki 0.101.7`, `sct 0.7.1` and `tokio-rustls 0.24.1`. No package was added. This also removes versions matched by the h2 and old webpki advisories. It does not remove all `http 0.2` usage or every duplicate crypto dependency. The experiment resolved dependency graphs; it did not compile or exercise network requests. [AWS HTTP client configuration](https://docs.aws.amazon.com/sdk-for-rust/latest/dg/http.html).

SeaORM's `mock` feature is enabled in production dependencies, but the observed `MockDatabase` use is in unit tests. Move its feature activation to development dependencies. I found no date/time model use requiring `with-chrono`; the temporary feature experiment removed `chrono` and `iana-time-zone` from the cache service graph. They remain in API/worker through PostHog. This is useful feature cleanup without rewriting the persistence layer. See [the manifest](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/crates/scope-postgres/Cargo.toml) and [mock tests](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/crates/scope-postgres/src/db/clerk_users.rs).

The web graph has several distinctly different opportunities:

| Package | Exclusive resolved snapshots | My recommendation |
| --- | ---: | --- |
| `react-doctor` | 228 | Remove the broad mandatory CLI after choosing the smaller checks that replace its required behavior. Do not assume the existing two boundary scripts replace all its rules. |
| `mermaid` | 116 | Keep if diagrams matter. If they do not, remove rendered diagrams and retain fenced text. Do not build a diagram engine. |
| `@pierre/diffs` | 22 | Keep. This is central product functionality, with highlighting and worker integration. |
| `posthog-js` | 10 | Replace its event-only use with a bounded adapter, preserving identity and privacy semantics. |
| `konsistent` | 5 | Replace the two configured conventions with checks using the existing TypeScript API. |
| `motion` | 4 | Replace the single collapse/expand animation with CSS or Web Animations. |
| `class-variance-authority` | 1 | Replace the handful of variant maps with typed functions. |
| `pagent` | 1 | Remove the staging-only feature by default; retain if remote investigations are intentional. |

These are graph-removal upper bounds computed from all 760 pnpm snapshots, including npm aliases, optional dependencies and peer contexts. They are not measured download, build-time or runtime savings. New replacements can add packages. Counts for different changes should not be added without recomputing the combined graph.

`react-doctor` is active and configured, including a supply-chain score requirement. Its graph includes Sentry/OpenTelemetry, installer/configuration utilities, lint engines and language-server packages. My objection is the amount of machinery installed for a mandatory check, not the usefulness of linting. Choose explicit Hooks rules and preserve the resource/observer checks; use dependency advisories and provenance checks for supply-chain policy. A smaller replacement still needs to be measured. [Current project configuration](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/web/doctor.config.json), [upstream documented behavior](https://github.com/millionco/react-doctor).

`konsistent` currently checks that routes export `Route` and that feature-page exports match their filenames. That is a good candidate for small local code. It also uniquely brings a TypeScript 5.9.3 copy alongside the web's 6.0.3. [Configured conventions](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/web/konsistent.json).

`motion` has two import sites for one animated discussion region. The replacement must preserve reduced-motion behavior, `inert`, focus handling and the callback that closes the composer. This is a small UI change, not merely deleting an import. [Animated region](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/web/src/features/requests/request-discussion-thread.tsx), [provider](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/web/src/features/requests/request-discussion-workbench.tsx).

The PostHog browser setup disables autocapture, replay, surveys, flags, external dependency loading and most automatic metadata. Scope already owns event context, privacy filtering and diagnostics. The server uses `ProductAnalyticsSink`, which is a suitable replacement boundary. Implement event delivery only, with explicit queue bounds, timeout/retry limits, shutdown flushing and accepted loss behavior. Preserve logout clearing, identity transitions, DNT, property filtering and `$process_person_profile`. Do not expand the replacement into a general analytics SDK. [Browser configuration](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/web/src/analytics/bootstrap.ts), [server adapter](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/crates/scope-product-analytics/src/transport.rs), [official capture API source](https://github.com/PostHog/posthog.com/blob/master/contents/docs/api/capture.mdx).

The analyzer already parses every source with TypeScript, then asks dependency-cruiser to parse and resolve the graph. A custom adapter can reuse the first pass and the TypeScript module resolver. It must still handle package aliases, `exports`, `imports`, JS/TS mode differences, missing targets, symlinks and snapshot containment. A regex import finder is not an acceptable substitute. Keep the current resolver until fixture comparisons show equivalent output. [Existing AST scan](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/dependency-analyzer/src/source-scan.mjs), [cruiser adapter](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/dependency-analyzer/src/cruiser.mjs), [upstream API](https://github.com/sverweij/dependency-cruiser/blob/main/doc/api.md).

## Git: keep it, standardize it, use more of it

Git's official latest stable source release is 2.55.0, which is also installed in this workspace. I would use that as the pinned server/check baseline, subject to rebuilding and testing the images. I would not select the 2.56 release candidate. API and worker use Ubuntu distribution packages; maintenance uses a PostgreSQL image plus distribution Git; checks inherit Git from a Rust image; the CLI uses the host's Git. None of those mechanisms proves they currently run the same version. I did not query live deployment binaries. [Git releases](https://git-scm.com/), [server image](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/deploy/railway/prebuilt.Dockerfile), [worker image](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/deploy/railway/worker.Dockerfile), [maintenance image](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/deploy/railway/maintenance.Dockerfile), [checks image](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/.scope/images/checks/Dockerfile).

The useful changes are specific to the code:

| Capability | Fit for Scope | Recommendation |
| --- | --- | --- |
| `merge-tree --write-tree --merge-base=<tree-ish>` | Server merge execution creates three synthetic commits solely to force the request base | Adopt after the existing merge suite and a few additional fixtures pass. This option arrived in Git 2.40, so this improvement is not exclusive to 2.55. |
| `cat-file --batch-command`, `--buffer`, `-Z` | Some blob-reading paths start one Git process per blob | Batch within an operation, retaining output bounds, cancellation, timeouts and repository/access isolation. The worker snapshot path already batches, so do not count it as new work. |
| `update-ref --stdin` transactions and expected old OIDs | Ref restoration/cleanup has repeated calls | Use where several ref changes must succeed together. This complements Scope database fences; it cannot replace them. |
| `diff-pairs -z` | Can render selected blob pairs after file selection and rename detection | Benchmark only if moving patch generation to Git becomes useful. The current API supplies old/new content to Pierre, so this is not an immediate renderer replacement. |
| Commit graphs, multi-pack indexes and newer incremental repacking | Worker restores packs and performs compaction | Benchmark on retained local materializations. Keep Scope's encrypted segment catalog and garbage-collection policy authoritative. |
| Linux fsmonitor, new in 2.55 | Potential local CLI status improvements for large working trees | Developer opt-in experiment, not a server requirement. Bare server repositories do not benefit from watching a working tree. |
| `git url-parse`, new in 2.55 | General Git remote syntax | No immediate change. Scope remote parsing deliberately validates HTTP URLs and origins; generic Git URL syntax is a different contract. |
| Reftable | Could improve very large ref sets | Defer until ref operations are measured as a bottleneck. Test creation, restore, hooks and concurrent writes before migration. |
| `git replay` and `git history` | New server-side replay/history operations | Do not base core request semantics on them yet. Their interfaces remain experimental. |
| SHA-256 repositories | Would change Git object identity throughout Scope | Do not enable as part of a Git version update. The code has explicit 40-character SHA-1 assumptions. Treat this as a separate domain/storage migration. |

The strongest simplification is at [merge execution](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/api/src/use_cases/request_merge.rs). Its existing rule is an explicit recorded request base. Replace the synthetic ancestry with that same explicit base, not a newly discovered merge base. The final two-parent merge commit remains necessary. [Git merge-tree documentation](https://git-scm.com/docs/git-merge-tree), [2.40 release notes](https://raw.githubusercontent.com/git/git/v2.40.0/Documentation/RelNotes/2.40.0.txt).

I compared the two preparations in seven disposable Git fixtures: disjoint edits, text conflicts, rename plus edit, delete plus modify, executable mode plus content, binary conflicts and adding empty files. All seven matched success/conflict status and conflict paths. All four successful merges produced the same tree. The text-conflict tree differed because marker labels include different input identities. Scope currently rejects that conflict result, but tests and diagnostics still need deliberate treatment. This is evidence for a focused refactor, not proof covering every Git merge scenario.

Batching fits [imported blob reads](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/api/src/git/import/artifacts.rs) and should reuse the process limits in `scope-git-process`. [Worker snapshots](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/worker/src/dependencies/snapshot.rs) already use `cat-file --batch`, and [import metadata](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/api/src/git/import/repo_io.rs) already uses `--batch-check`. A long-lived global process could accidentally mix repositories or access scopes; begin with operation-scoped batches. `-Z` provides NUL framing for both directions; `-z` only changes input framing. [Git cat-file](https://git-scm.com/docs/git-cat-file).

For refs, do not substitute `update-ref --batch-updates` where all-or-nothing behavior is required. That option allows individual invalid updates to fail while others apply. Use the transaction commands with expected old OIDs. Git ref transactions still cannot commit atomically with PostgreSQL. [Git update-ref](https://git-scm.com/docs/git-update-ref).

Git 2.55 includes merge-base/revision traversal, pack ingestion, index-pack and incremental MIDX improvements that could benefit this workload without new application code. Actual value needs measurements of cold restoration, warm reads, request merges and compaction. `diff-pairs` separates file selection from patch generation, but requires full object IDs and upstream path filtering. [2.55 release notes](https://raw.githubusercontent.com/git/git/v2.55.0/Documentation/RelNotes/2.55.0.adoc), [diff-pairs](https://git-scm.com/docs/git-diff-pairs).

Reftable migration has explicit concurrency restrictions, and the documentation still disallows migration of repositories with worktrees. That is another reason to avoid a blanket conversion. `git replay` identifies itself as experimental. [Git refs](https://git-scm.com/docs/git-refs), [Git replay](https://git-scm.com/docs/git-replay).

For adoption, add one Git version owner for server images and startup/readiness diagnostics, then update all relevant images together. Declare the host CLI minimum and reject unsupported versions clearly. Add no fallback implementation for older Git. Run existing push, merge, private/public projection, request-ref, restoration and CLI tests with the selected version. Keep Git patch-level changes separate from changes to merge behavior.

## Upgrade findings that need attention

I queried OSV for 1,351 unique registry package/version pairs from the four lockfiles and the four Python direct pins. All queries completed. Seventeen package/version pairs matched advisory records. These are version matches, not seventeen demonstrated vulnerabilities in Scope. Duplicate GHSA/RustSec identifiers can describe the same issue. The CSV links every match and lists fixed versions across the corresponding advisories.

| Dependency | Observed version | Action and qualification |
| --- | --- | --- |
| `rustls` | 0.23.41 in both Rust locks | Update to at least 0.23.45. Upstream reports incorrect TLS 1.3 encryption-level handling. The advisory explicitly says the handshake transcript remains authenticated; do not describe this as arbitrary handshake forgery. |
| `h2`, old `rustls-webpki` | 0.3.27 and 0.101.7 | Remove the old AWS transport graph. Modern copies already exist. webpki name-constraint issues have issuance preconditions; the CRL panic matters only where CRLs are used. |
| `anyhow` | CLI 1.0.102 | Update to 1.0.104. The workspace's 1.0.103 already contains the reported fix. The issue requires context plus mutable downcasting. |
| `webbrowser` | CLI 1.2.1 | Update to 1.2.4. The injection issue requires particular Unix BROWSER templates and attacker-controlled non-HTTP URLs. Inspect Scope's authorization URL contract as well. |
| `event-listener` | 5.4.1 | Update to at least 5.4.2 through the SQLx graph. Upstream identifies unsound cross-thread behavior for particular tagged listeners. |
| `lru` | CLI 0.18.0 | Update to at least 0.18.2. Panic-safety issue with stored key destruction; no application exploit established. |
| `rsa` | 0.9.10 | Retain with use-specific tracking. The advisory concerns RSA decryption timing. Observed Scope operations verify RS256/ES256 Clerk signatures and sign/verify EdDSA grants, not RSA decryption. |
| `nitro` | 3.0.260311-beta | Update to a tested current beta, at least 3.0.260429-beta for these fixes. Affected wildcard proxy/redirect routeRules were not found in project configuration. |
| `fast-uri` | 3.1.2 | Refresh to at least 3.1.6 within Ajv's supported range. Scope uses Ajv for generated contract validators, not as its URL authorization parser. |
| `postcss`, `nanoid` | 8.5.15 and 3.3.12 | Refresh to at least 8.5.23 and 3.3.18. Primarily build-time paths here; advisories involve source maps or unusual generator arguments. |
| `brace-expansion`, `browserslist`, `baseline-browser-mapping` | 5.0.6, 4.28.2, 2.10.37 | Refresh to at least 5.0.9, 4.28.7 and 2.11.0. Mainly tooling input/resource-exhaustion issues; parent updates may resolve them. |
| `js-yaml`, `lodash-es` | 4.2.0 and 4.17.23 | Refresh to at least 4.3.2 and 4.18.0 where parent ranges permit. Lodash-es is under Mermaid's parser stack. A dependency occurrence alone does not establish a vulnerable call. |

The highest-priority Rust runtime observation is supported by [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html). CLI browser handling is documented in [RUSTSEC-2026-0257](https://rustsec.org/advisories/RUSTSEC-2026-0257.html). Nitro's affected configurations and fixes are documented in its [proxy advisory](https://github.com/nitrojs/nitro/security/advisories/GHSA-5w89-w975-hf9q) and [redirect advisory](https://github.com/nitrojs/nitro/security/advisories/GHSA-9phm-9p8f-hw5m). For the remaining records and exact ranges, use the per-package [advisory CSV](dependency-audit-2026-09-23/advisory-matches.csv).

DOMPurify 3.4.15 is worthwhile even though the currently queried versions did not match an OSV record. Its release specifically improves XML clobbering defenses, relevant to SVG rendering. Updating only the direct declaration can leave nested copies behind; the lock also contains PostHog's 3.4.13 copy. [DOMPurify release](https://github.com/cure53/DOMPurify/releases/tag/3.4.15).

Node is inconsistent across shipping paths: CLI bundles already pin 24.21.0, while checks and worker images pin 24.18.0 and the web image floats on `node:24-bookworm-slim`. Align on a reviewed Node 24 LTS patch and immutable image digests. 24.21.0 includes OpenSSL, Undici and root certificate updates. Keep Node 24 rather than adopting Node 26 just to follow `latest`. Also align `@types/node` to 24.x. [Node 24.21.0 release](https://nodejs.org/en/blog/release/v24.21.0).

TypeScript is the clearest case where `latest` is the wrong automatic action. The registry reports 7.0.2, but TypeScript 7 has no stable compiler API. Both the analyzer and web architecture scripts import that API. Keep their 6.x dependency. A native compiler performance trial would be separate and would retain the API dependency, so it is not a dependency-reduction project. [TypeScript 7 announcement](https://devblogs.microsoft.com/typescript/announcing-typescript-7-0/).

Reqwest 0.13.5 is worth a focused migration because direct callers use 0.12.28 while PostHog brings 0.13.4. Review the renamed TLS feature, provider/root selection, proxy behavior, blocking runtime use and timeouts. Updating declarations without aligning features misses much of the benefit. [Reqwest release notes](https://github.com/seanmonstar/reqwest/releases).

SeaORM 2.0.3 and SQLx 0.9.0 are available, but they belong in one planned persistence migration. Raw connection use, locks, transaction behavior and migrations need checks together. Fix the current graph's transitive advisories first instead of making a major ORM upgrade a prerequisite. [SeaORM migration guide](https://www.sea-ql.org/blog/2026-01-12-sea-orm-2.0/).

PostgreSQL test environments use 15 and 16 while the maintenance image explicitly targets production PostgreSQL 18.6. Make the primary integration and recovery test lane match production 18.6. Older test majors do not provide useful compatibility coverage for a pre-alpha product with one deployed database version. This is a test-environment alignment, not a claim that production needs a major upgrade. [PostgreSQL version policy](https://www.postgresql.org/support/versioning/).

Media dependencies deserve a separate image refresh. FFmpeg is pinned at Debian 5.1.9 and libheif at 1.15.1 with a Debian revision. Upstream FFmpeg offers 5.1.10 on the old branch and 9.0.2 on its current branch; libheif's 1.23.5 release includes security fixes. Do not infer exposure solely from upstream numbers because distributions backport fixes. Compare the actual image SBOM/vendor advisories, then rebuild and run the existing codec self-test and output assertions. A major codec change can alter orientation, animation, color, thumbnails and resource use. [FFmpeg releases](https://ffmpeg.org/download.html), [libheif releases](https://github.com/strukturag/libheif/releases), [media Dockerfile](https://github.com/scope-vcs/scope-vcs/blob/dab2cca3c3d1bce76c750e6a1dbf81a7041ab152/media-worker/Dockerfile).

## What I would keep outsourced

Keep Git as the Git engine. There is no direct `git2` or `gix` dependency to remove. Adding either now would introduce another implementation alongside the subprocess engine. Scope's domain rules, visibility projections and durable content catalog remain its own responsibilities.

Keep the S3 SDK for multipart storage. The small-object store uses a narrower signed HTTP implementation, while Git storage needs multipart completion, aborts and streaming. The duplication is worth reviewing at the transport-owner boundary, but extending handwritten S3 code to replace the SDK is a poor first bet. S3 can return HTTP 200 and then an error inside the completion response; its SDK handles that condition. Credentials, retry classification, cancellation and incomplete uploads add more work. Consolidate ownership without forcing the large SDK into every small service or casually replacing durable-storage behavior. [S3 completion semantics](https://docs.aws.amazon.com/AmazonS3/latest/API/API_CompleteMultipartUpload.html).

Keep SeaORM and its shared SQLx driver. Direct SQLx calls handle session-owned advisory locks, notifications and fencing beneath the ORM. They are not redundant CRUD frameworks. The lockfile contains MySQL/SQLite-related optional packages, but the inspected Linux normal/build trees do not include those drivers. Do not count dormant lock entries as shipped database engines.

Keep Clerk, JWT verification, native credential stores, RustCrypto primitives, DOMPurify and URL parsers. These own complex security-sensitive behavior. A maintained library is preferable to Scope becoming the maintainer of another sanitizer, cryptographic primitive or identity system.

Keep Schemars, ts-rs and Ajv together. They provide runtime schemas, TypeScript declarations and compiled runtime validators respectively. The Rust generation dependencies are optional, and removing runtime validation because TypeScript exists would weaken API boundaries.

Keep Radix dialogs/tooltips and the terminal libraries. Focus management, keyboard navigation, terminal cleanup and Unicode widths have real edge cases. The small savings from replacing them are outweighed by regression risk. Font and icon files remain external dependencies even if copied into the repository. The adapted shadcn components are already owned source with recorded upstream notices.

## Delivery order and limits of this audit

Start with a focused compiler/advisory update and AWS feature cleanup. Validate both Rust locks, all CLI platforms, S3 multipart failure/abort behavior, credential discovery, database notifications and recovery. Existing image scans should run on rebuilt images, not just source lockfiles.

Next, standardize Git without changing application behavior. Then implement the explicit-base merge simplification in its own change. Use existing clean/conflicting merge tests plus rename, mode, binary, explicit-base and failure-path cases. Keep request authorization and publication checks in Scope.

Then handle small dependency removals one at a time: two-rule convention checker, style variants and the discussion animation. UI changes need browser inspection. Choose whether to retire Pagent, preserve Mermaid and replace the mandatory React Doctor gate before removing their behavior. Analytics and resolver rewrites need their own bounded contracts and equivalence fixtures.

The review used manifest and source inspection, Cargo metadata, Linux normal/build trees, a complete pnpm snapshot traversal, registry queries, official release notes, upstream advisory records and two disposable experiments. It did not benchmark production, compile the proposed dependency set, inspect authenticated deployed images or prove every advisory reachable. The transitive CSV gives each locked package a disposition through its parents; it is not a line-by-line code audit of 1,562 third-party packages. Python transitive dependencies and OS image contents are not fully locked by the checked-in requirements, so a complete deployed SBOM remains an image-level task. No product behavior or dependency declaration was changed by this review.
