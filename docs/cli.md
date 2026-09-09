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

For an existing Git repository, run `scope init --name repository`, inspect and
commit the generated rules files, then publish explicitly to main:

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

Show and preview inspect tracked and untracked worktree files, excluding ignored
files. Preview is an offline comparison; it does not compare server configuration
or the committed tree that a push will publish.

## Contribute and review

```sh
scope request start fix-parser --title "Handle quoted paths"
# Edit files, then git add and git commit.
scope request push
scope request submit --yes
scope request checks
```

Maintainers can inspect requests and their checks from any directory by naming
the repository explicitly. Checkout requires a local Git repository:

```sh
scope --repo owner/repository request list --state open --limit 20
scope --repo owner/repository request show --request fix-parser
scope --repo owner/repository request diff --request fix-parser
scope --repo owner/repository request checks --request fix-parser
scope request checkout --request fix-parser
scope request merge --request fix-parser --yes
```

Request targets accept a name or a `req_` ID. Inside a request checkout, commands
can infer the current request. List also supports `--audience public|private`
and `--search TEXT`. Diff prints text hunks for server-visible files in the server-selected review revision. Use
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

Start uses a committed workflow and local Git source. Discovery, inspection,
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
scope --json --non-interactive --repo owner/repository request list --state open
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
stored `scope.apiUrl`. `scope status` and `scope doctor` report the selected
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
names. Pull requests execute native Linux x64, macOS Apple Silicon, and Windows
x64 lanes. Releases also execute macOS Intel. Linux ARM64 and Windows ARM64 are
build-only lanes and are labeled accordingly. Native lanes exercise the version
and license commands plus installation through the real download service. They
also run existing tests for credential-key isolation by API URL, Git credential
request handling with an injected token reader, and browser callback validation.
The Unix-only exchange-file permissions test runs on Linux and macOS.

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

The download service remains for now. `.github/workflows/publish-cli.yml`
stages the service, all six artifacts, and generated checksums into one Railway
deployment. Railway uses `/readyz` to reject a release missing any required
download, and the service generates installers using its configured public URL.
Deleting it requires a static-host replacement that preserves the public install
and download URLs, publishes the complete release together, generates installers
with the correct URL, and supplies equivalent deployment verification. No such
replacement is wired into the deployment graph today. Keep this small service
until that cutover is implemented and verified.
