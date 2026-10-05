# Repeated agent mistakes

This audit starts at `d859a514`. It inspected the latest 250 commit subjects,
followed resource and test corrections further back, read the latest 100 GitHub
review comments, both recorded reverts, `AGENTS.md` history, and the comments
removed by `74946610`. Counts below are independently evidenced occurrences,
not an exhaustive count of every bug. A review and its repair count once.

| Class | Evidence count | Disposition |
| --- | ---: | --- |
| Frontend data owned by individual callers | 7 | Close the remaining route publication bypass with private ownership and a boundary check. |
| Tests that do not observe their claimed failure | 5 | Repair swallowed callback assertions and reject the pattern. |
| Required mutation effects left to individual callers | 3 | Existing lifecycle and use-case owners already centralize the evidenced effects; retain their behavior tests. |
| Comments reintroduced after removal | 3 | Merged [PR #525](https://github.com/scope-vcs/scope-vcs/pull/525) adds the shared policy check. |
| Setup failure text normalized inconsistently | 2 | Make validated text a domain type. |
| Host-dependent test concurrency exhausts PostgreSQL | 2 | Bound concurrency in the shared backend runner. |

## Frontend data ownership

Evidence: `393e6264` retained repository resources, `ef2ab587` retained list
pages, `031634c3` retained logs, `21b831e4` moved reply reads into their resource,
`ad8e47bf` centralized viewer resets, `0b51a291` repaired account session
revalidation, and `8fdf4e63` shared native run reads. The
[account review](https://github.com/scope-vcs/scope-vcs/pull/505#discussion_r4186015485)
showed a late loader restoring a revoked session. The
[run review](https://github.com/scope-vcs/scope-vcs/pull/514#discussion_r4186570971)
showed independently invalidated owners reading the workflow catalog twice.

The generic cache already rejects obsolete load completions. A route calling
`write` bypassed that protection and had to copy viewer and generation checks.
The account resource now keeps its mutable store private and owns loader
admission, revocation, and viewer reset. Its hook checks admission again when
hydration effects run. Pinned request changes also publish through their
existing feature owner. The obsolete raw account exports are deleted.

Level: architecture, backed by types and lint. Private exports prevent the old
account import from compiling. Other resources still have legitimate mutation
and invalidation consumers, so making every resource private would require a
larger rewrite. The route boundary rejects their direct `write`, `seed`, and
`writeIfUnchanged` calls and names the owner module to use. No suppression was
added. Existing effect-read restrictions remain in force.

Proof: the historical account route from `083d3703` is rejected by the new
boundary check. Owner tests hold a loader across revocation, reject other
viewers, and reject a same-user handoff after viewer reset. A rendered browser
check revokes Laptop, delivers its old loader, leaves and reopens the list:
only Desktop remains, with zero additional reads. The 390px view does not
scroll horizontally.

## Tests that swallow their own failures

Evidence: `8c12da49` replaced a panic swallowed by a spawned task with observed
resolver calls; `319e4540` moved ineffective CSRF checks to the server boundary;
`9e7b9ebe` repaired retained tests' negative controls; `bb57eaba` tested real API
and storage entry points; the
[hydration review](https://github.com/scope-vcs/scope-vcs/pull/514#discussion_r4186570985)
replaced preset counters and client mounting with real server hydration.

Two remaining cache tests put `assert.fail` inside a loader. The resource
correctly catches loader failures, so an unexpected read could run without
failing the test. Discussion pagination had the same sentinel pattern.
Tests now assert observed call counts after awaiting the operation. Adjacent
callback sentinels in streaming, recovery, rendering, and settings tests use
the same pattern, preserving their existing result assertions.

Level: lint and behavior tests. Production must catch network and rendering
errors; changing that behavior to rescue assertions would be incorrect.
Types cannot prove that a callback was never invoked. The test-source check
rejects `assert.fail` sentinels in injected callbacks, including imported aliases
and direct function references, and points to `node:test`'s `mock.fn()` and
`mock.callCount()`. Direct assertions in tests and top-level assertion helpers
remain valid. This is a focused guard, not a proof that every possible test is
meaningful.

Proof: the check rejects the original cache tests from `393e6264`. Two isolated
mutations demonstrate the behavioral difference:

| Mutation to the real cache owner | Old assertion | Repaired assertion |
| --- | --- | --- |
| Retry an unchanged revision after a load error | Passes despite the extra read | Fails: call count is 1, expected 0 |
| Require a retained entry even when an oversized result is visible | Passes despite the extra read | Fails: call count is 1, expected 0 |

The restored owner passes all its tests. The stream cancellation test now also
includes an active-stream control: replacing `runRepoEventStream` with an empty
async function fails on the missing connection and interruption. Mutation runs
used temporary compiled copies or a child-process preload, so no mutation was
left in the checkout.

Inspection covered the complete changed test files and their production
owners. This was not an exhaustive audit of Rust, deployment, or browser test
quality; existing historical repairs were sampled as evidence, not recounted
as new fixes.

## Setup failure messages

Evidence: `ced5ed47` repaired NUL-containing run text that PostgreSQL could not
store; `4ac1ae0a` repaired worker truncation by characters instead of bytes,
which rejected multibyte provider messages and lost capacity retries. The
worker previously carried a comment about capacity retry policy while still
constructing unchecked strings outside the domain.

Level: types. Provider text is external input, so architecture cannot eliminate
malformed strings. `SetupFailureMessage` has private storage, a strict input
constructor, a normalizing producer constructor, and validating deserialization.
Run conclusions, capacity-retry entry points, and persisted terminal reasons
require it. Raw `String`
construction and the old voluntary normalization helper are removed. The HTTP
wire format and error message remain unchanged.

Proof: the historical construction
`AttemptConclusion::SetupFailed { exit_code: 1, message: "setup\0failed".to_string() }`
fails with E0308, expected `SetupFailureMessage`, found `String`. Domain tests
cover NUL, whitespace, multibyte truncation, and valid/invalid serialization.
The worker's multibyte capacity-retry regression passes through persistence.
No additional instruction to remember normalization is needed.

## Host-dependent test concurrency

Evidence: [issue #524](https://github.com/scope-vcs/scope-vcs/issues/524)
records an earlier default-parallelism run with 116 failures from PostgreSQL
connection exhaustion. The first full backend gate in this audit reproduced
the same cause: 428 API tests passed and 64 failed, with reserved connection
slot errors and pool acquisition timeouts. The failure and repair are tracked
in [issue #527](https://github.com/scope-vcs/scope-vcs/issues/527).

Level: architecture of the test runner, backed by an executable regression.
`dev/checks/backend` owns test concurrency and sets four threads, matching the
four-core CI host. Per-test pools can consume eight connections; inheriting
the larger local host's thread count exceeded PostgreSQL's connection budget.
Domain types cannot constrain the test harness's process-wide concurrency.
This needs no contributor instruction or per-machine override.

Proof: the regression runs the actual gate with a recording Cargo executable
and an inherited 128-thread setting. Every test command must receive four
threads, and an empty gate fails because it produces no receipt. Running the
same assertion against the historical gate from `b97bc731` observes 128 for
all three test commands and fails; the repaired gate observes four and passes.
The repaired gate's full API suite passes all 492 tests without connection
exhaustion. The regression runs in the shared policy gate locally and in CI.

## Existing prevention and scope

Required side effects were corrected in `102fe663`, `c2136f91`, and `2fcfcd4f`.
`request_lifecycle_effects::persist_lifecycle_mutation` owns terminal persistence,
while request use cases own the evidenced committed refresh publication.
Those repairs are already on the baseline. This audit does not claim that
all future side effects can be inferred or that every refresh path was audited.

Comment removal required `74946610`, `73e56b91`, and `fdcb64de`; the latter two
removed comments arriving through merges. PR #525 subsequently merged the
comment scanner and its regression tests into the shared policy gate. Neither
the June auth revert `26b78b76` nor bounded-I/O revert `e46a012e` alone establishes
another repeated class.

The follow-up review checked the four subsequent merges: #507 changes CLI
request comparison, #521 recovers auto-merge checks, #526 moves landed-request
completion into the main-push transaction, and #525 enforces the comment rule.
They share no changed files with this patch and merge cleanly. The combined
tree passes the policy gate, including the new comment scanner, and
`cargo check --workspace --all-targets --locked`.

## Shared commands

`./dev/checks/web` runs the behavior tests, `pnpm check:resource-boundary`
(including both guards), type checks, hook/convention checks, and production
build. `.github/workflows/scope-web-ci.yml` uses that same entry point.
`./dev/checks/backend` owns workspace tests and clippy, including the domain
and worker regressions. `.github/workflows/rust-workspace-checks.yml` uses it.
`./dev/checks/policy` is identical locally and in CI.

Focused checks for these changes are `pnpm --dir web check:resource-boundary`,
`cargo test -p scope-domain --locked`, and
`cargo test -p worker --locked multibyte_capacity_rejection_records_retry`.
These are subsets of the shared gates, not alternate implementations.
The full web, policy, and repaired backend gates passed locally, including
workspace tests, both API feature suites, and workspace all-target clippy.
The focused Rust tests and historical negative controls also passed their
expected success/failure assertions. CI uses these same shared entry points.
