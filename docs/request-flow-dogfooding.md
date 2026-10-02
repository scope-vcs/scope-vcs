# Dogfooding the request flow

Dogfooding should expose weaknesses in Scope's request lifecycle and verify their
fixes. A successful mirrored push proves only the part of the lifecycle it
exercised. GitHub remains authoritative for this repository's merges and releases.

## Choose the next exercise from the missing evidence

Use a real contribution for ordinary setup, push, revision, review, and recovery
checks. Use the seeded local stack or a designated native Scope repository for
merge and disruptive failure scenarios. Never use this repository's mirrored main
to test native merge: it may receive only commits already merged on GitHub main.

| Scenario | Evidence to inspect | Existing executable coverage |
| --- | --- | --- |
| Fresh linked worktree | Doctor separates absent optional visibility from broken state; pull initializes it without moving an origin-tracking branch or overwriting edits | CLI inspection, repo config, pull tests |
| Start interrupted after remote creation | Recover the same request and branch; no duplicate request or lost commits | CLI request workflow recovery test |
| Push to another request | Target head advances; current branch identity and upstream remain unchanged | CLI request workflow recovery test |
| Contributor and maintainer review | Draft visibility, checkout, diff, discussions, and access agree | Two-actor contribution integration |
| Checks and revised heads | Contributor heads reach `scope/requests/<id>` only after maintainer approval; the request shows the GitHub checks for the same commit that a pull request would run; required checks block merge; a result for an older head does not count; a new head gets fresh results | Two-actor contribution integration and API request-check and GitHub check tests |
| Auto merge | Authorization binds to a revision; cancellation, failed checks, and newer revisions stop it | Two-actor contribution integration and API auto-merge tests |
| Native merge | Correct files reach main; private files remain private; conflicting main changes stop merge | Two-actor contribution integration and API auto-merge tests |
| Close and interrupted responses | Inspect state before retry; authorized repeat close distinguishes closed/merged; concurrent close has one transition; unauthorized actors remain denied | Two-actor contribution integration, domain and API close tests |
| Browser/CLI agreement | Inspect the same request and revision in both, including navigation and reopening after changes | Seeded web request smoke tests plus a paired browser/CLI exercise |

Run the CLI contribution integration against a seeded stack with
`SCOPE_API_URL=http://localhost:8080 dev/checks/integration cli`. Its two actors use
separate sessions. The checks scenario deliberately leaves its checks pending when
no runner is present; it proves approval and merge gating, not that workflows ran.
API auto-merge tests cover completed checks and conflict handling separately.

Checks on this repository's requests come from its GitHub Actions workflows on
`scope/requests/<id>`. Until this repository is connected to GitHub, record
request checks as setup-blocked. Do not start Scope runs.

In a repository connected to GitHub, a public request's checks run on a check
commit: the contribution merged onto private main, pushed to
`scope/requests/<request id>` only after a maintainer approves. Confirm on GitHub
that the branch holds the private files as well as the contribution, and that the
request view never shows that commit's id. A contribution that conflicts with
private code should report that conflict as a configuration error and push
nothing.

## Record what happened

For each relevant scenario, record **passed**, **failed**, **setup-blocked**, or
**unexercised**. Include the environment, CLI/server revision when known, actor
role, request/head, expected result, actual result, and the next action. Keep local
fix verification separate from behavior observed with the released CLI and hosted
service. A setup failure is useful evidence, but it does not count as a lifecycle
pass. A pending or queued check is not a successful check.

A useful PR note can be short:

```text
Environment / versions:
Scenario and role:
Request / revision:
Expected → observed:
Result: passed | failed | setup-blocked | unexercised
Recovery or remaining gap:
```

## Change the amount and kind of dogfooding when evidence warrants it

- Fix repeated setup blockers once, then rerun the blocked scenario. Repeating the
  same setup failure on more PRs adds no request-lifecycle evidence.
- When ordinary mirroring passes, move effort to uncovered roles, revised checks,
  native merge, conflicts, and interrupted operations. More successful mirror
  notes alone do not justify calling the request flow robust.
- Repeat a scenario after its behavior changes, after a regression, or when a
  different environment introduces uncertainty. Choose repetitions for a concrete
  failure risk, not a target number of PRs.
- Keep a proposed change only when it repairs an observed failure or closes a
  meaningful evidence gap. Revise or remove steps that only make the reporting
  ritual easier without improving request behavior or its verification.
- Stop expanding an exercise once its remaining uncertainty is covered by a more
  direct test. Keep failures and unexercised scenarios visible in the PR note.
