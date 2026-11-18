Scope is pre-alpha. Allow migrations, but do not add backward compatibility layers or preserve obsolete interfaces.

Maintainer means an owner or member.

Do not modify the root `README.md` unless the user explicitly asks.

When useful, delegate independent coding tasks to medium-level subagents. Announce how they will be used, separate worktrees for larger independent work, and review the combined result.

Keep core concepts, rules, transitions, invariants, and required side effects in durable domain code. Outer layers translate inputs and outputs, call domain behavior, persist or render results, and surface errors. Keep sources of truth singular, side effects explicit, and modules owned by behavior. Refactor unclear ownership; delete speculative abstractions and obsolete paths.

Frontend server data must reuse existing resource and cache owners, survive navigation and reopening within the same viewer and access scope, and refresh from relevant changes without blanking valid data. Check reuse and invalidation rather than introducing component-owned fetch lifecycles.

Do not write comments or documentation files. Code is the foundation: express intent through names, types, structure, and tests. When code cannot carry the context, fix the underlying issue instead of explaining it. Requests and pull requests are the record of why a change was made. The only comments are machine-read text: tool directives, generated files, and digest-locked migrations. The only prose files are these agent instructions, agent skills, the root README, and legal and security notices.

Around 1,000 lines, audit a file's responsibilities and modularize unless there is a good reason to keep it together.

Proceed with behavior-preserving refactors. Involve the user before choosing behavior beyond the agreed request; do not reopen behavior already authorized.

Pushes and merges to main do not deploy changes. Releases happen separately.

Do not re-run a failed check to pass it without a repair until the failure is recorded: open a `release-flake` issue, or comment on the open one for that failure, with the test or step name, the run link, and the failure output. An open `release-flake` issue for the same failure means it has recurred; fix its cause before finishing the task and close the issue with that fix.

Two kinds of first failure get one re-run before they are recorded. A local check that failed while another checkout was building or testing on the same machine is re-run once that work has finished. A step that failed reaching an outside service, such as a package mirror, a registry, or Railway's API, is re-run unchanged. Record either one if it fails again.

Set autoreview's timeout to 15 minutes with `--engine-timeout-seconds 900`. Do independent work while it runs.

## Scope contribution rules

This repository dogfoods Scope. Scope is the source of truth for review, merge
gating, and main. GitHub runs Actions and receives main from Scope, and releases
build from that main.

Deliver each change as one Scope request: `scope request start`, then
`scope request push` for each revision, then `scope request submit`. Merge it in
Scope with native merge, the merge queue, or auto-merge once its required checks
and reviews pass. Do not open GitHub pull requests, merge on GitHub, or push to
GitHub's main; Scope opens a pull request itself only when a review bot needs one.

Checks on this repository's Scope requests come from its GitHub Actions
workflows, which run when Scope pushes the request to `scope/requests/<id>`.
Re-run failed checks on GitHub.

GitHub's main only ever receives Scope's main. A change delivered through GitHub
instead makes Scope stop publishing main until the two are reconciled, so a Scope
failure blocks delivery: open an issue for it and wait for the fix rather than
working around Scope.

## Rule enforcement

When an operator corrects a repeat, repair the owning architecture or type in
that change, then add a check if the wrong code can still be written. Update
this table for remaining rules. Remove instructions made unnecessary by an
unrepresentable state.

| Rule requiring contributor judgment | Enforcement or explicit limit |
| --- | --- |
| Keep request header rows aligned and use SVG control icons. | `request-detail-header` owns row layout and compact action sizing. The request-header browser spec compares cropped screenshots at desktop, phone, and both sides of its container breakpoint, including a long branch and open Changes menu. Baseline changes require visual review. |
| Keep domain policy and required effects in their owners. | Crate dependencies: `dev/checks/policy` and `check-rust-boundaries.mjs`. Lifecycle and refresh behavior: Rust domain/store/use-case tests. Placement of new policy still requires review. |
| Reuse frontend resource owners; keep retained data scoped and refreshed. | `createCachedResource` owns retention and viewer resets. `createRepoResource` requires every repository resource to name the topics that invalidate it, and `topicsOf` fails typecheck for an unhandled event kind. `pnpm check:resource-boundary` rejects effect-owned reads, direct route publication, and repository features that call `createCachedResource` directly; resource behavior tests cover races. Whether a resource names the right topics still requires review. |
| Assert callback behavior outside the code that can catch its failures. | `pnpm check:resource-boundary` checks test sources with `callback-assertions.mjs`; use `mock.fn()` and assert `mock.callCount()` after awaiting work. |
| Write tests that catch real breaks and stay fast. | `dev/checks/policy` runs `check-test-waits.mjs`: real-clock test waits of 5 s or more, or of a computed number of seconds, fail. Duplication, signal, and setup cost still require review with the `scope-tests` skill. |
| Keep pull request CI fast. | `daily-monitors.yml` opens an issue when a job's median over its last 10 successful pull request samples since the latest workflow change on main exceeds 7.5 minutes. Job concurrency, build profiles, and cache saves in workflows still need review, and so does a slow new test. |
| Keep the run page's log fast. | The GitHub runs component spec pins the requests made before log text shows. Production `run_log_timing` percentiles are an analytics report with no automated budget, so a new slow path still needs review. |
| Keep a checkout without `deploy/` and `bench/` checkable. | Scripts that read the deployment manifest, `deploy/`, `bench/`, or a release workflow live in `deploy/scripts` and run from `dev/checks/ops`; `.github/scripts` and the other gates read none of them. `check-gates.test.mjs` rejects a gate command that names an operations file and pins `./dev/check` skipping `ops` without `deploy/scripts`. A script that reads an operations file indirectly still needs review. |
| Audit large files and separate responsibilities. | `dev/checks/policy` runs `check-source-size.mjs`: ownership audit at 700 lines, hard cap at 1,000. |
| Preserve generated contracts and digest-locked migrations. | `dev/checks/contract` and `check-migration-immutability.mjs`. |
| Write no comments. | `dev/checks/policy` runs `check-comments.mjs` and its regression tests. Doc comments and `SAFETY` notes fail like any other comment. |
| Fix lint findings instead of suppressing them. | Rust lints live in `[workspace.lints]`; `check-rust-boundaries.mjs` keeps the CLI and CLI-staged manifests' tables identical. `clippy::allow_attributes` rejects `#[allow]`, `clippy::expect_used` rejects `.expect()` outside tests, and an `#[expect]` fails once nothing triggers it. `check-comments.mjs` rejects `@ts-`, `eslint-`, and `oxlint-` directives outside generated files. Web lint runs `oxlint --type-aware` with `web/.oxlintrc.json`. Whether an `#[expect]`, `.ok()`, or `.expect()` is justified still requires review. |
| Keep dependencies used and from known sources. | `dev/checks/backend` and `dev/checks/cli` run `cargo shear` and `cargo deny check bans sources`. |
| Write no documentation files. | `dev/checks/policy` runs `check-docs.mjs` and its regression tests: Markdown and other prose files, READMEs below the root, and anything under `docs/` fail. |
| Let a dependency trust exclusion or ignored advisory lapse on its expiry date. | `dev/checks/policy` runs `check-trust-exclusions.mjs` and its regression tests: an entry in `trustPolicyExclude` fails from the expiry date recorded for its package, and an advisory in `auditConfig.ignoreGhsas` fails without a recorded reason, expiry, and approver, or from that expiry. Recording an expiry for a new exclusion still requires review. |
| Avoid compatibility layers in pre-alpha; maintainer means owner or member. | Domain authorization tests cover access. Compatibility and terminology choices still require review. |
| Do not edit root README without explicit authorization; involve the operator in new behavior. | Operator context and review; no source-only check can establish authorization. |
| Record failed checks before retrying without a repair, after one re-run for a first failure from machine load or an outside service. | Release-flake issue workflow; no local retry interceptor. |
| Deliver every change as a Scope request merged in Scope; GitHub's main only receives Scope's main. | Scope refuses to publish main once GitHub's main has commits Scope lacks. Opening a GitHub pull request or merging on GitHub is not blocked and still requires review. |
| Keep releases separate from pushes and merges. | GitHub release workflows. |
| Review with the configured timeout and verify before committing. | Autoreview and shared `dev/checks` commands; review invocation remains an agent responsibility. |

The new checks have no suppression syntax and this change adds no exceptions.
A future exception must be attached to the offending line with its reason,
expiry date, and a named human's approval; do not add a blanket allowlist.
