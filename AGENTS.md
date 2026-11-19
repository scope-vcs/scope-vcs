Scope is pre-alpha. Allow migrations, but do not add backward compatibility layers or preserve obsolete interfaces.

Maintainer means an owner or member.

Do not modify the root `README.md` unless the user explicitly asks.

Keep rules, transitions, invariants, and required side effects in domain code. Outer layers translate inputs, call domain behavior, and persist or render results.

Frontend server data goes through `createRepoResource`; do not fetch from components.

Do not write comments or documentation files. Express intent through names, types, structure, and tests; requests are the record of why.

Proceed with behavior-preserving refactors. Involve the user before choosing behavior beyond the agreed request; do not reopen behavior already authorized.

Pushes and merges to main do not deploy. Releases happen separately.

Record a failed check before re-running it without a repair: open a `release-flake` issue, or comment on the open one, with the step name, run link, and failure output. A first failure caused by another local build or test, or by an outside service such as a registry or Railway's API, gets one unchanged re-run first. A failure with an open `release-flake` issue has recurred: fix its cause and close the issue with that fix.

Run autoreview with `--engine-timeout-seconds 900`.

## Delivery

Deliver each change as one Scope request: `scope request start`, `scope request push` for each revision, then `scope request submit`, and merge it in Scope. Never open a GitHub pull request, merge on GitHub, or push to GitHub's main. Re-run failed checks on GitHub. If Scope itself fails, open an issue and wait for the fix.

## Rules

When corrected on a repeat mistake, fix the owning type or architecture and add a check so it cannot recur. An exception to a check goes on the offending line with its reason, expiry date, and a named human's approval.
