---
name: release
description: Use when the user asks to release, ship, or deploy Scope to production.
---

# Release

Production changes only through the Release workflow on main. Never deploy with Railway directly or from a branch.

1. If `gh run list -w release.yml -s in_progress -L 1` shows a running release, watch that run instead of dispatching another.
2. Dispatch: `gh workflow run release.yml --ref main -f scope=changed`. Use `-f scope=all` only when the user asks to redeploy everything.
3. Find the run with `gh run list -w release.yml -e workflow_dispatch -L 1 --json databaseId,url,headSha` and confirm `headSha` is main's tip.
4. Start `gh run watch <id> --exit-status` as a background command and wait on it with `t3_background_wait`.
5. Report the run link, released SHA, components the plan selected, and the result. If the plan selected nothing, say nothing changed since the last release.

If it fails, report the failing job and the relevant lines of `gh run view <id> --log-failed`, then follow ci-failure-triage. Do not roll back or work around the workflow without the user.
