Scope is pre-alpha. Allow migrations, but do not add backward compatibility layers or preserve obsolete interfaces.

Maintainer means an owner or member.

Do not modify the root `README.md` unless the user explicitly asks.

When useful, delegate independent coding tasks to medium-level subagents. Announce how they will be used, separate worktrees for larger independent work, and review the combined result.

Keep core concepts, rules, transitions, invariants, and required side effects in durable domain code. Outer layers translate inputs and outputs, call domain behavior, persist or render results, and surface errors. Keep sources of truth singular, side effects explicit, and modules owned by behavior. Refactor unclear ownership; delete speculative abstractions and obsolete paths.

Frontend server data must reuse existing resource and cache owners, survive navigation and reopening within the same viewer and access scope, and refresh from relevant changes without blanking valid data. Check reuse and invalidation rather than introducing component-owned fetch lifecycles.

Around 1,000 lines, audit a file's responsibilities and modularize unless there is a good reason to keep it together.

Proceed with behavior-preserving refactors. Involve the user before choosing behavior beyond the agreed request; do not reopen behavior already authorized.

Pushes and merges to main do not deploy changes. Releases happen separately.

Set autoreview's timeout to 15 minutes with `--engine-timeout-seconds 900`. Do independent work while it runs.

<!-- scope:rules:start -->
## Scope contribution rules

Read and follow `.scope/RULES.md` before
making or submitting changes.
<!-- scope:rules:end -->
