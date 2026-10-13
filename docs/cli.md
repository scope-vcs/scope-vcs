# Scope CLI

Use `scope --help` or append `--help` to a command for its arguments. Scope works
with Git commits. Commit your changes before publishing them or starting a run.

## First use

Install Scope using the command shown on scopevcs.com. Installers verify the
binary's SHA-256 checksum before replacing an existing installation. On Linux
and macOS, the install directory must already be on `PATH`. To choose a user
installation directory:

```sh
mkdir -p "$HOME/.local/bin"
export PATH="$HOME/.local/bin:$PATH"
export SCOPE_INSTALL_DIR="$HOME/.local/bin"
```

Rerun the installer to update. On Windows, the installer adds the installation
directory to the current process and user `PATH` when needed. Check the installed
build with `scope --version`.

```sh
scope login
scope clone owner/repository
cd repository
scope status
scope doctor
```

For an existing Git repository, run `scope init --name repository`, choose which
files are public, then publish explicitly to main:

```sh
scope visibility edit
scope visibility validate
scope visibility show
scope push --main
```

`scope push --main` publishes the current commit to the repository's main branch.
The `--main` flag confirms that destination. A request has its own publication
command, `scope request push`. Read `scope status` before publishing if the branch
or repository context is unfamiliar.

Visibility configuration belongs to the Git worktree's local Scope state. It is
not a committed configuration file. `scope visibility show` prints the rules;
`scope visibility explain path/to/file` explains a path's visibility. Use
`scope visibility edit` for the interactive editor. To compare a proposed JSON
configuration file against local rules without saving it:

```sh
scope visibility preview --config proposed-config.json
```

`scope visibility log` lists the repository's visibility changes on Scope,
newest first, including pushes and merges that changed visibility. Unlike the
local commands above, it needs `scope login` and network access. Pages hold up to
50 changes; pass the printed `--before` cursor to see older ones.

A fresh linked worktree can contribute to requests before it has local visibility
state. `scope doctor` reports that absence as informational. Run `scope pull` to
initialize the missing state from the repository (public contributors receive
private-by-default local rules). Pull preserves existing visibility edits and
leaves a branch tracking another remote, such as GitHub's `origin`, at its current
commit. Invalid or conflicting partial visibility state still needs repair.

Show and preview inspect tracked and untracked worktree files, excluding ignored
files. Preview is an offline comparison; it does not compare server configuration
or the committed tree that a push will publish.

## Contribute and review

```sh
scope request start fix-parser --title "Handle quoted paths"
# Edit files, then git add and git commit.
scope request push
scope request submit --yes
scope request ci
```

`scope request start` creates a branch from main named after the request. To
turn a branch that already holds commits into a request, run it with
`--current-branch`: the branch keeps its name and its upstream, and must already
contain Scope's main. When a push to main carries an open request's head, Scope
records that request as merged.

`scope request push` also updates a request after you rebase or amend it. The
push replaces the request head only if Scope still has the head this checkout
last saw. If someone else pushed to the request since then, the push stops with
"Someone else updated this request. Fetch it and try again." Run `scope pull`,
bring their commits into your branch, and push again. A request branch must
still contain the main commit it is based on. Each push moves that base forward
to the newest main commit the branch contains. If public main moves before a
public request merges, rebase onto it or merge it, then run
`scope request push`.

Maintainers can inspect requests and their CI from any directory by naming
the repository explicitly. Checkout requires a local Git repository:

```sh
scope --repo owner/repository request list
scope --repo owner/repository request show --request fix-parser
scope --repo owner/repository request diff --request fix-parser
scope --repo owner/repository request ci --request fix-parser
scope request checkout --request fix-parser
scope request merge --request fix-parser --yes
scope request merge --request fix-parser --auto --yes
```

Private maintainers can create a child of an Open request in the same full view:

```sh
scope request start parser-ui --base parser-core
scope request start parser-tests --base parser-ui --current-branch
```

Omitting `--view` inherits the parent's exact view. Start pins its current saved
revision and head, fetches that head with the CLI session, and creates the child
branch there. Adoption keeps the current branch and upstream and requires that
branch to contain the selected parent head. Checkout keeps each child's own
request branch and upstream; parent objects use `refs/scope/parents/`.

`scope request diff` lets the server select the comparison. Private requests
default to the complete layer between the saved parent pin and the child's head;
public and restricted views retain visible push comparisons until aggregate
review is available for those views. Use `--comparison push` for one push or
`--comparison commit --revision <revision-id> --commit <full-oid>` for one commit.
Show, status and push report the named ancestor path, saved review pin, current
parent revision and visible child names. The saved pin stays the review base
when the parent advances. `--json` includes the server's authorized
`result.stack_navigation`, with ancestors from root to parent and a first page
of children. A `children.next_cursor` marks more children; human output calls
out that cursor. Inspect a visible child with `request show --request <name>`.
Unavailable dependencies have an opaque label without a parent name or link.

Depth describes the request's ancestry, with a main-based root at depth 1.
Forks can share a depth; it is not a position in a linear stack. A child waits for
its parent to land. An extension that still contains the saved pin is informational:
ordinary pushes retain that pin and the child keeps its existing review and CI.
A rewritten or closed parent blocks landing until the author repairs the child.

Restack replays only the child's commits after its saved review base. It keeps
its request association and upstream and leaves remote publication explicit:

```sh
scope request checkout --request parser-ui
scope request restack
git diff <selected-base-full-oid>...HEAD
scope request push
scope request diff
```

With no `--onto`, restack selects the current saved parent revision, or current
main for a main-based request. Parent extensions do not require restacking; use
it voluntarily to include fix-ups. Recover from a closed parent or choose a new
parent explicitly:

```sh
scope request restack --onto main
scope request restack --onto parser-core-v2
```

The initial command requires a clean worktree and index, an attached Scope
request branch, no competing Git operation, and sole worktree ownership of that
branch. It fetches the selected base through the CLI session and preserves the
request's required main ancestry. A parent that predates that ancestry must be
updated first, or select main. Merge commits in the child's replay range require
manual restacking with preserved merge history; the helper refuses them before
rewriting the branch.

Git owns conflict progress. Stage resolutions, then continue or abort:

```sh
git add <resolved-files>
scope request restack --continue
scope request restack --abort
```

A conflict returns exit 5 with `recovery` fields naming the original head and
base, selected base, backup ref, and continuation, abort and publication
commands. Successful JSON output uses `command: "request.restack"` and
`result.status: "ready_to_publish"` or `"aborted"`. Abort verifies that Git
restored the original head before clearing the pending selection. Backup refs
remain under `refs/scope/restacks/` for inspection.

If Git completed before the helper was interrupted, `--continue` reports the
current head and retained selection. `--abort` refuses to reset a completed
rebase blindly; inspect the backup and current HEAD before manual recovery.
After inspecting a completed replay, `scope request push` consumes the pending
selection only for its original repository, remote and destination request.
A stale publication leaves that selection available for recovery. If main or
the selected parent moved, discard only the stale selection after inspecting the
completed replay:

```sh
scope request restack --discard-selection --json
git fetch scope +refs/heads/main:refs/remotes/scope/main
git rebase --onto refs/remotes/scope/main <reported-selected-base-full-oid>
scope request push --base main
```

For a request base, fetch its current branch into `refs/scope/parents/`, replay
from the reported previous selected OID onto that fetched head, and use
`scope request push --base <request-name-or-id>`. Verify the fetched head matches
`scope request show --request <request-name-or-id>` before replaying.
`--discard-selection` requires the original branch and no active Git operation.
It preserves HEAD, resolved files, refs, and the backup. JSON reports
`result.status: "selection_discarded"` with the preserved head and old selected
base. It cannot combine with `--onto`, `--continue`, or `--abort`. An ambiguous
same-head publication receipt remains available for its original retry; this
action does not discard that receipt or authorize a server update.

Manual Git restacks can publish an explicit base without the helper:

```sh
scope request push --base-revision <current-parent-revision-id>
scope request push --base parser-core-v2
scope request push --base main
```

`--base-revision` selects the same parent's current saved revision. `--base`
selects an Open request in the same private full view or current main. These
options are mutually exclusive. A selection supplied after a native restack
must agree with its retained selection. Ordinary pushes without a selection
retain their parent pin. Main-based requests continue advancing their review
base through contained main history; the source branch never supplies a new
parent.
Targeted pushes keep the source branch's association and upstream.

Changed-head publication saves the head and base together. A fresh Draft may
restack before its first saved revision, but an unchanged head cannot publish a
new base without saved child evidence. Keep the pending selection, commit the
child's own changes, then run `scope request push` to save its first revision.
Same-head base changes use a committed revision-context receipt. After a lost response, retry
the same head and selection so the saved operation key recovers the original
receipt. Recovery returns Conflict if a newer publication prevents confirmation
of the current intended head and base.

Every push to a request evaluates CI for its current revision. In a repository
connected to GitHub, CI uses the required check names from repository
settings, answered by GitHub Actions on the pushed commit; any other repository
runs its own `.scope/runs` workflows. A maintainer's push starts CI at
once: Scope starts its runs, or sends the revision to GitHub. An outside
contributor's push waits until a maintainer runs
`scope request ci --approve --head <head>`, naming the head they reviewed;
a newer head is refused. `scope request ci` shows the evaluation,
each job's provider and state, and where its logs are: the run id for
`scope run logs` or the GitHub page. For GitHub CI it also shows whether the
revision is waiting for approval, being sent, sent, or failed to send, and warns
before approving a request that changes GitHub workflow files. It also shows what
merging still waits on: an open request merges only after CI passes. A
GitHub check with no run yet is pending. If evaluating a push failed, the request
cannot merge until CI is evaluated; `scope request ci` and
`scope request merge` both evaluate CI again. See
[the GitHub App guide](github-app.md) for how GitHub CI runs.

When CI is still running, a maintainer can authorize the current request
revision to merge later with `scope request merge --auto --yes`. CI awaiting
maintainer permission still requires `scope request ci --approve`; enabling
auto-merge does not approve CI. The command returns after the server saves the authorization; a
successful exit does not by itself mean the request has merged. `scope request show` reports the authorizer,
authorized head, state, and what auto-merge is waiting for. A new request push
ends that authorization. Cancel an active authorization with
`scope request merge --cancel-auto --yes`; `--auto` and `--cancel-auto` cannot be
used together. Existing request targeting and `--json` output work for both
operations.

Authorize each layer of a private stack separately:

```bash
scope request merge --request layer-a --auto --yes
scope request merge --request layer-b --auto --yes
scope request merge --request layer-c --auto --yes
```

Each authorization names that layer’s exact saved revision and head. Child
layers report “Waiting for the parent request to land” until their parent lands;
authorizing a parent does not authorize its children. Parent extensions preserve
the child’s authorization and review pin. A new child revision, rewrite, or
reparent ends its authorization; rewritten or closed parents and blocked
ancestors stop it. Landing still requires the authorized head’s required checks to pass and the
current native merge gate to allow it. Inspect each layer with
`scope request show --request <name>` before authorizing it again.

`scope request list` is your attention queue, grouped the way the web sidebar
groups it: Needs you, Waiting on others, and Unclaimed. Use
`--section set-aside` or `--section done` for requests you set aside or that
have closed or merged. `--search TEXT` matches titles and descriptions on the server,
and `--limit` caps each section. Maintainers move an open request through the
queue. Each row carries authorized parent context even when the parent is
outside the selected section, search or page; that context does not add a queue
item or change ordering, limits or cursors. Actionable children remain in the
default queue when their parent is off-page. Parent extensions, satisfied
dependencies and parent landing preserve attention. A new hard dependency
blocker, including a rewrite, close or conflict, can bring children back to
attention, while a future snooze keeps its deadline.

```sh
scope request claim --request fix-parser    # become its reviewer
scope request release --request fix-parser  # stop reviewing it
scope request wait --request fix-parser     # set aside until someone replies
scope request settle --request fix-parser   # set aside until new activity
scope request snooze --request fix-parser --for tomorrow
scope request restore --request fix-parser  # back to Needs you
```

Snooze takes `hour`, `tomorrow`, or `next-week`; the last two land at 09:00
local time, next week on Monday. If the request changes between loading it and
the change, the command fails, prints the new activity, and asks you to run it
again.

Request targets accept a name or a `req_` ID. Inside a request checkout, commands
can infer the current request. Diff prints text hunks for server-visible files in the server-selected review revision. Use
`--revision ID`, then `--commit OID`, then `--path FILE` to inspect a specific file's
old and new contents in a commit. Incomplete or truncated server results are
explicitly labeled. Checkout requires a clean tree. An existing local branch must
already belong to the same request and can only fast-forward; checkout never
resets it or discards work.

Read Markdown from stdin without shell interpolation:

```sh
scope request edit --description-file - < description.md
scope request discussion start --body-file - < review.md
```

## Workflows and runs

Native runs execute `.scope/runs` workflows on Scope's own runner. They are
available only to repositories whose owner the server allowlists. Other
repositories run CI on GitHub Actions through
[the GitHub App](github-app.md).

```sh
scope run workflows
scope run start checks --no-watch
scope run list --workflow checks --limit 20
scope run show RUN_ID
scope run logs RUN_ID --job test
scope run watch RUN_ID --timeout 600
scope run cancel RUN_ID
scope run retry RUN_ID --no-watch
```

Start uses a committed workflow and local Git source. Runs that a request started
carry the `request` trigger and list alongside the rest. Discovery, inspection,
logs, cancellation, and retry support `--repo owner/repository` without a checkout.
List supports `--after CURSOR` for the next page. Watching defaults to 1,800
seconds, bounds reconnect attempts, and ignores duplicate log positions.
A timeout reports a resume command with `--after LOG_POSITION`. Ctrl-C stops
watching without canceling the remote run. Failed, canceled, or lost runs return
exit code 5; an interrupted connection or timeout returns 6.

## Automation and context

Use `--non-interactive` to prevent terminal prompts and browser login. Sign in
explicitly before running authenticated automation. `scope login --headless`
provides the browser handoff for a machine without a local browser.

```sh
scope --json --non-interactive --repo owner/repository request list --section active
scope --json --non-interactive push --main --no-review
scope --json --non-interactive run start checks --no-watch
scope --json --non-interactive run watch RUN_ID --timeout 600
```

`--no-review` explicitly skips the interactive visibility editor and uses the
local visibility rules. It does not skip publication validation. Submission,
closing, and merging require their own `--yes` confirmation when no prompt is
available.

Finite JSON commands return a versioned result document. Watching runs emits
JSON Lines events for logs and status, including an initial receipt when starting
or retrying with watching enabled. Git progress and other diagnostics may appear
on stderr. On failure, the final stderr line is the JSON error document; parse
that line's error code and retryability rather than matching English messages.
Shell completions output shell source and do not accept `--json`. The interactive
visibility editor also does not accept `--json`.

| Exit code | Meaning |
| --- | --- |
| 0 | Success |
| 1 | Unexpected failure |
| 2 | Invalid arguments or missing local prerequisites |
| 3 | Authentication required or expired |
| 4 | Permission or policy refusal |
| 5 | State conflict, missing resource, or unsuccessful terminal run |
| 6 | Temporary failure or watch timeout |

Endpoint selection takes the first available value in this order:

1. `--api-url URL`.
2. Runtime `SCOPE_API_URL`.
3. Runtime `SCOPE_API_PUBLIC_URL`.
4. The checkout's Git configuration value `scope.apiUrl`.
5. The API URL embedded at build time, preferring `SCOPE_API_URL` over
   `SCOPE_API_PUBLIC_URL`, or Scope's production default when neither was embedded.

Repository operations reject an endpoint that conflicts with the checkout's
stored `scope.apiUrl`. A checkout without that value whose Scope remote lives at
another origin than the selected endpoint is reported with both recoveries:
re-point the remote and sign in again, or store the remote's origin as
`scope.apiUrl`. `scope status` and `scope doctor` report the selected
endpoint; use `--offline` to inspect local state without contacting Scope. Both
commands return exit code 0 when they successfully emit a report, even when
`result.healthy` is false. Automation must inspect `healthy` and `diagnostics`.
Offline checks report remote checks as `not_checked`, so a healthy offline report
does not establish that authentication or the server is available. When
multiple Scope repositories are configured locally, select `--repo owner/repository`
or a command's `--remote NAME` rather than relying on remote naming conventions.

Generate shell completions with `scope completions SHELL`. Use its help to list
the supported shells.

## Recovery

Start with `scope doctor` for setup problems and `scope status` for branch,
publication, and request context. Fix the reported prerequisite before retrying.

If request creation or pushing fails after a side effect, the error identifies the
request, failed stage, whether pushing completed, and an exact retry command.
Use that command, usually `scope request push --remote REMOTE --request ID`.
Pushing to a named request updates that request without changing the current
branch's request association or Git upstream. If request start could not save
local branch metadata, push from that branch, then run the reported
`scope request checkout --remote REMOTE --request ID --branch BRANCH` command to
restore its association. Checkout only adopts an unassociated existing branch
when its HEAD matches the request head.
Inspect the request with `scope request show --request ID` if needed. Do not rerun
`request start` after the draft exists. An unwanted draft
can be closed with `scope request close --request NAME --yes`.

If publication reports configuration drift, inspect `scope visibility show`,
resolve the visibility rules with `scope visibility edit`, and inspect them with
`scope visibility show` before retrying `scope push --main`. A stale review needs a fresh review.
If a run watch times out, resume watching the reported run and log position;
starting a new workflow would create another run.

## Distribution checks and hosting

`cli/distribution/targets.json` owns the six release targets and their artifact
names. It also sets `max_artifact_bytes`, and packaging fails when any target's
archive exceeds that cap. Pull requests execute native Linux x64 and Windows x64
lanes. Releases also execute macOS Apple Silicon and macOS Intel. Linux ARM64 and Windows ARM64 are
build-only lanes and are labeled accordingly. Native lanes exercise the version
and license commands plus installation through the real download service. They
also run existing tests for credential-key isolation by API URL, Git credential
request handling with an injected token reader, and browser callback validation.
The Unix-only exchange-file permissions test runs on Linux and macOS.
Pull requests skip the native lanes when their CLI changes are limited to
`crates/scope-domain/` and `crates/scope-api-contract/`. The Linux CLI checks
still build and test the CLI against those crates, and releases run every lane.

The installer check uses a temporary directory containing spaces. It verifies
first installation, replacement of an old binary, command discovery on `PATH`,
checksum rejection, and preservation of the installed binary after a corrupt
download. POSIX checks also reject a directory outside `PATH`. Windows checks
restore the runner's original user `PATH` afterward. The full Rust suite runs
on Linux because authentication fixtures currently use Linux's file session
store. Native checks never read or write a runner's actual login credentials.
They compile the native keychain integration but do not verify Keychain or Windows
Credential Manager CRUD. The current store setup installs the production store
on first use and has no injectable store constructor; a real-store test would
mutate the runner's credential store and depend on its unlocked desktop session.
Add an injectable boundary before claiming isolated native credential-store tests.

The download service remains for now. Its `scope-cli-service` binary comes from
the native Linux x64 matrix lane, which uploads it as
`cli-service-release-<sha>` on releases; nothing else builds the CLI a second
time. `.github/workflows/publish-cli.yml`
stages the service, all six artifacts, and generated checksums into one Railway
deployment. Railway uses `/readyz` to reject a release missing any required
download, and the service generates installers using its configured public URL.
Deleting it requires a static-host replacement that preserves the public install
and download URLs, publishes the complete release together, generates installers
with the correct URL, and supplies equivalent deployment verification. No such
replacement is wired into the deployment graph today. Keep this small service
until that cutover is implemented and verified.
