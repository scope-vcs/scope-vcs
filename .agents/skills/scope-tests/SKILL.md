---
name: scope-tests
description: Use when writing, changing, reviewing, or deleting tests in Scope.
---

# Scope tests

PR #726 halved CI time by deleting about 75 low-signal tests and fixing a few slow ones. Do not write those tests again.

## Before adding a test

Answer all three. If one has no answer, do not add the test.

1. What realistic change to production code turns it red?
2. Which existing test already catches that change? Extend that test or its table instead of writing a near-copy. Prove each rule once at its owning layer, plus at most one end-to-end test for a distinct risk such as transport, auth, or persistence.
3. What does it cost per CI run? Databases, spawned processes, git subprocesses, browsers, and real time are expensive.

## Do not write

- checks of what types already guarantee: field presence, plain serde round-trips, constructors returning their input;
- restated constants, copied lists or manifests, or expected values computed by the code under test;
- getters, defaults, one-line mappings, or framework behavior;
- assertions that a mock received what the test gave it;
- "renders" or "route exists" checks that a stronger test already passes through;
- names that promise more than the assertions check.

## Keep tests fast

- Never wait on the real clock. Use `#[tokio::test(start_paused = true)]`, an injected `now`, or inputs placed at the edge of the deadline. `dev/checks/policy` rejects fixed waits of 5 s or more; treat anything near 1 s as a defect too.
- Wait for the event, not a duration. Do not use `waitForTimeout` or fixed-interval polling in smoke specs.
- A fake server that must stall awaits `std::future::pending()`.
- Build expensive fixtures, such as a migrated schema, once per test and isolate cases with rolled-back transactions.
- Run independent scenarios in parallel with isolated state, and many test files in one `node --test` process.

## Deleting tests

Slow is a reason to speed a test up, not to delete it. Delete only when you can name the surviving test that fails for the same break, and list each pair in the PR. Never delete without that pair for authorization and visibility, persistence and migrations, git protocol and ref updates, request and merge state machines, or deploy safety.

## Proof

- A regression test fails on the pre-fix code.
- After rewriting or speeding up a test, break the code under test once, confirm the intended assertion fails, and restore it.
- Run the owning gate through scope-checks.
