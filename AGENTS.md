Scope is pre-alpha. Allow migrations, but do not add backward compatibility layers or preserve obsolete interfaces.

Maintainer means an owner or member.

Do not modify the root `README.md` unless the user explicitly asks.

When useful, delegate independent coding tasks to medium-level subagents. Announce how they will be used, separate worktrees for larger independent work, and review the combined result.

Keep core concepts, rules, transitions, invariants, and required side effects in durable domain code. Outer layers translate inputs and outputs, call domain behavior, persist or render results, and surface errors. Keep sources of truth singular, side effects explicit, and modules owned by behavior. Refactor unclear ownership; delete speculative abstractions and obsolete paths.

Frontend server data must reuse existing resource and cache owners, survive navigation and reopening within the same viewer and access scope, and refresh from relevant changes without blanking valid data. Check reuse and invalidation rather than introducing component-owned fetch lifecycles.

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
