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
| Checks and revised heads | Only maintainers approve; pending checks block merge; the new head needs fresh approval | Two-actor contribution integration and API request-check tests |
| Auto merge | Authorization binds to a revision; cancellation, failed checks, and newer revisions stop it | Two-actor contribution integration and API auto-merge tests |
| Native merge | Correct files reach main; private files remain private; conflicting main changes stop merge | Two-actor contribution integration and API auto-merge tests |
| Close and interrupted responses | Inspect state before retry; authorized repeat close distinguishes closed/merged; concurrent close has one transition; unauthorized actors remain denied | Two-actor contribution integration, domain and API close tests |
| Browser/CLI agreement | Inspect the same request and revision in both, including navigation and reopening after changes | Seeded web request smoke tests plus a paired browser/CLI exercise |

Run the CLI contribution integration against a seeded stack with
`SCOPE_API_URL=http://localhost:8080 dev/checks/integration cli`. Its two actors use
separate sessions. The checks scenario deliberately leaves jobs queued when no
runner is present; it proves approval and merge gating, not successful execution.
API auto-merge tests cover completed checks and conflict handling separately.

Public request checks must be exercised through an actual public clone. Workflow
files are private, so that clone cannot carry the workflow definitions. Public
requests select the accepted main workflow catalog on the server, retain the
selected definitions for their head, and run them against the public request's
snapshot after maintainer approval. Private requests use definitions at their
head. A fixture that inserts an approval record directly cannot prove this source
selection works.

## Record what happened

For each relevant scenario, record **passed**, **failed**, **setup-blocked**, or
**unexercised**. Include the environment, CLI/server revision when known, actor
role, request/head, expected result, actual result, and the next action. Keep local
fix verification separate from behavior observed with the released CLI and hosted
service. A setup failure is useful evidence, but it does not count as a lifecycle
pass. A queued check is not a successful check.

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
