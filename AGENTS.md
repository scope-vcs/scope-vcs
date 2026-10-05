Scope is pre-alpha. Allow migrations, but do not add backward compatibility layers or preserve obsolete interfaces.

Maintainer means an owner or member.

Do not modify the root `README.md` unless the user explicitly asks.

When useful, delegate independent coding tasks to medium-level subagents. Announce how they will be used, separate worktrees for larger independent work, and review the combined result.

Keep core concepts, rules, transitions, invariants, and required side effects in durable domain code. Outer layers translate inputs and outputs, call domain behavior, persist or render results, and surface errors. Keep sources of truth singular, side effects explicit, and modules owned by behavior. Refactor unclear ownership; delete speculative abstractions and obsolete paths.

Frontend server data must reuse existing resource and cache owners, survive navigation and reopening within the same viewer and access scope, and refresh from relevant changes without blanking valid data. Check reuse and invalidation rather than introducing component-owned fetch lifecycles.

Do not write comments. Code is the foundation: express intent through names, types, structure, and tests. When code cannot carry the context, fix the underlying issue instead of explaining it. The only exceptions are machine-read text: `///` docs on API contract types (they generate the TypeScript contract), `// SAFETY:` on `unsafe` blocks, tool directives, generated files, and digest-locked migrations.

Around 1,000 lines, audit a file's responsibilities and modularize unless there is a good reason to keep it together.

Proceed with behavior-preserving refactors. Involve the user before choosing behavior beyond the agreed request; do not reopen behavior already authorized.

Pushes and merges to main do not deploy changes. Releases happen separately.

Do not re-run a failed check to pass it without a repair until the failure is recorded: open a `release-flake` issue, or comment on the open one for that failure, with the test or step name, the run link, and the failure output. An open `release-flake` issue for the same failure means it has recurred; fix its cause before finishing the task and close the issue with that fix.

Set autoreview's timeout to 15 minutes with `--engine-timeout-seconds 900`. Do independent work while it runs.

## Scope contribution rules

This repository dogfoods Scope. GitHub remains the source of truth for merges,
CI gating, deploys, and releases.

Maintainers mirror every branch they push to GitHub, every PR they open, and
every merge to main to their `scope` remote, following the dogfood-scope skill
from their personal agent skills. The skill is not part of this repository, and
contributors without it or without a `scope` remote have nothing to mirror. A
Scope failure never blocks GitHub delivery; it is noted in the PR description.

Checks on this repository's Scope requests come from its GitHub Actions
workflows on `scope/requests/<id>`. Do not start Scope runs.

Main on Scope only ever receives commits already on GitHub's main.

## Rule enforcement

When an operator corrects a repeat, repair the owning architecture or type in
that change, then add a check if the wrong code can still be written. Update
this table for remaining rules. Remove instructions made unnecessary by an
unrepresentable state. Evidence and proof for this audit are in
[docs/agent-corrections.md](docs/agent-corrections.md).

| Rule requiring contributor judgment | Enforcement or explicit limit |
| --- | --- |
| Keep domain policy and required effects in their owners. | Crate dependencies: `dev/checks/policy` and `check-rust-boundaries.mjs`. Lifecycle and refresh behavior: Rust domain/store/use-case tests. Placement of new policy still requires review. |
| Reuse frontend resource owners; keep retained data scoped and refreshed. | `createCachedResource` owns retention and viewer resets. `pnpm check:resource-boundary` rejects effect-owned reads and direct route publication; resource behavior tests cover races. |
| Assert callback behavior outside the code that can catch its failures. | `pnpm check:resource-boundary` checks test sources with `callback-assertions.mjs`; use `mock.fn()` and assert `mock.callCount()` after awaiting work. |
| Audit large files and separate responsibilities. | `dev/checks/policy` runs `check-source-size.mjs`: ownership audit at 700 lines, hard cap at 1,000. |
| Preserve generated contracts and digest-locked migrations. | `dev/checks/contract` and `check-migration-immutability.mjs`. |
| Avoid ordinary code comments. | No local enforcement yet; separate PR #525 adds it. The exceptions above remain the intended policy. |
| Avoid compatibility layers in pre-alpha; maintainer means owner or member. | Domain authorization tests cover access. Compatibility and terminology choices still require review. |
| Do not edit root README without explicit authorization; involve the operator in new behavior. | Operator context and review; no source-only check can establish authorization. |
| Record failed checks before retrying without a repair. | Release-flake issue workflow; no local retry interceptor. |
| Mirror GitHub delivery to Scope, keep Scope main downstream, and do not start Scope runs. | `dogfood-scope` skill and contribution workflow; not enforced by compilation. |
| Keep releases separate from pushes and merges. | GitHub release workflows. |
| Review with the configured timeout and verify before committing. | Autoreview and shared `dev/checks` commands; review invocation remains an agent responsibility. |

The new checks have no suppression syntax and this change adds no exceptions.
A future exception must be attached to the offending line with its reason,
expiry date, and a named human's approval; do not add a blanket allowlist.
