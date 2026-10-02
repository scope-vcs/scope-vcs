# Scope GitHub App

A maintainer connects a Scope repository to the project's GitHub repository
by installing the Scope GitHub App. Scope then pushes each request revision
to a branch of that repository, GitHub Actions runs the project's workflows
on it, and the results decide whether the request can merge. One app serves
every Scope repository on a server.

## Registering the app

Create a GitHub App owned by the organization that runs Scope, with:

- **Callback URL**: `https://<web origin>/github/setup`. Scope sends this as
  the OAuth `redirect_uri`, so it must match exactly.
- **Request user authorization (OAuth) during installation**: off. With it on,
  GitHub disables the Setup URL. Connecting uses the standard OAuth web flow
  instead, which always returns its code with Scope's state.
- **Expire user authorization tokens**: GitHub's default is fine. Scope uses a
  user token for one setup and never stores it.
- **Setup URL**: `https://<web origin>/github/setup`, so a maintainer who
  installs the app from the connect page comes back to finish. Turn on
  **Redirect on update** so adding repositories to an existing installation
  also comes back.
- **Webhook URL**: `https://<api origin>/v1/github/webhooks`, with a random
  webhook secret.
- **Repository permissions**: Contents read and write, Workflows read and
  write, Checks read, Actions read, Metadata read.
- **Subscribed events**: Check run, Check suite, Workflow run, Repository,
  Installation, Installation repositories. Installation events are always
  delivered to GitHub Apps; the others are chosen on the registration page.
  Repository events report a repository made public or private.
- **Where can this app be installed**: any account.

Generate a private key and a client secret on the app's page. The callback
address is built from the origin of the page that started connecting when
that origin is `SCOPE_APP_ORIGIN` or one of `CLERK_AUTHORIZED_PARTIES`, and
from `SCOPE_APP_ORIGIN` otherwise. Any other page origin is refused. Register
a callback URL for every web origin maintainers connect from.

## API configuration

| Variable | Value |
| --- | --- |
| `SCOPE_GITHUB_APP_ID` | The numeric app ID |
| `SCOPE_GITHUB_APP_SLUG` | The app's URL name, as in `github.com/apps/<slug>` |
| `SCOPE_GITHUB_APP_PRIVATE_KEY` | The PEM private key. Line breaks may be written as `\n` |
| `SCOPE_GITHUB_APP_CLIENT_ID` | The app's client ID |
| `SCOPE_GITHUB_APP_CLIENT_SECRET` | A client secret. It also signs the setup state, so rotating it ends setups in progress |
| `SCOPE_GITHUB_WEBHOOK_SECRET` | The webhook secret |

With none of them set, GitHub connections are off and repository settings say
GitHub is not configured on this server. Setting only some of them stops the
API at startup.

## Connecting

1. A maintainer chooses Connect GitHub in the CI section of repository
   settings, or on the Runs page of a repository with no runs, no workflows of
   its own and no link. The page they started from is kept in session
   storage, and the setup page returns there once the repository is
   connected, when it is a page of that repository; otherwise it opens the
   repository's settings.
   `POST /v1/repos/{owner}/{repo}/github/authorize`, sent with the page's
   origin, returns GitHub's OAuth URL for the app with a signed `state` naming
   the Scope repository, the maintainer, and a ten-minute expiry.
2. GitHub returns to `/github/setup` with a code and the state. The page sends
   both to `POST /v1/github/setup`. The API checks the state, that the same
   person is signed in and is still a maintainer, and exchanges the code for a
   user token. Through that token it lists the installations of the app the
   person can access and, in each, the repositories they can push (push,
   maintain or admin; read access is not enough). It returns those
   repositories and a signed grant recording each one's installation. The
   user token is not stored.
3. The maintainer picks a repository and `POST /v1/repos/{owner}/{repo}/github`
   stores the link after checking the grant and confirming with an
   installation token that the app still reaches the repository.

When the repository is missing from the list, the page links to the app's
install page and offers Check again, which restarts the OAuth step. Before
leaving for the install page it remembers the Scope repository in session
storage. GitHub's Setup URL brings the maintainer back to `/github/setup`
without a code or state; the page then restarts the OAuth step for the
remembered repository. Installation ids GitHub adds to that URL are never
used: installation ids are guessable, and only the user token and the
installation token decide what can be connected.

A Scope repository has at most one link, and a GitHub repository is connected
to at most one Scope repository at a time. Disconnecting in settings removes
the link. Uninstalling or suspending the app, or removing the repository from
the installation, keeps the link as disconnected with the reason, and
settings offer to reconnect. Unsuspending does not reconnect by itself.
Connecting, or reconnecting, sends again the tested commit of every open
request whose GitHub checks are started.

### Public GitHub repositories

Everything Scope pushes to a public GitHub repository is public there,
private requests and private files included. Connecting one needs a
maintainer who can change file visibility, and the connect call must carry
`acknowledge_public`: the setup page asks the maintainer to confirm it.
Whether a repository is public is what GitHub reports when connecting, not
what setup listed.

A connected repository can become public later. A `repository` delivery
saying it was made public or private makes Scope ask GitHub which it is now,
and every push of a private request asks GitHub first as well. A repository
that became public receives no private request: their checks become
configuration errors and their pushes give up. Settings say so, and a
maintainer who can change file visibility can allow private requests with
`POST /v1/repos/{owner}/{repo}/github/public-confirmation`, which sends the
held requests again. Settings show a public repository as public, and a
private request's checks say that its changes are public on GitHub.

## Request checks

A repository with a GitHub link, connected or disconnected, answers its
request checks on GitHub. Any other repository runs its own `.scope/runs`
workflows. A repository never uses both.

### Running workflows on requests

Add a push trigger for Scope's branches to each workflow that should run on
requests. Existing triggers stay:

```yaml
on:
  pull_request:
  push:
    branches: ['scope/**']
```

GitHub runs the workflow files in the pushed commit. Workflows that read
`github.event.pull_request` find it empty on a branch push and need a
fallback. Once connected, the CI section of repository settings shows
this trigger.

### Testing the connection

Test connection in the CI section
(`POST /v1/repos/{owner}/{repo}/github/setup-check`, maintainers only) sends
the repository's current main to `scope/setup-check` with the same push jobs
requests use. A repository keeps its latest test in
`scope_github_setup_checks`, so the result stays across reloads and
navigation; the connection response (`GET /v1/repos/{owner}/{repo}/github`)
carries it. A second test waits until the first ends. Main holds private
files, so it goes only where a private request may: a GitHub repository that
became public is not tested until a maintainer allows private requests there.

If GitHub refuses the push, for example because a branch ruleset restricts
creating `scope/**` branches, the test fails at once and shows GitHub's
answer. Otherwise a background pass reads, every 30 seconds, the workflow runs
GitHub started on `scope/setup-check` for that commit
(`GET /repos/{owner}/{repo}/actions/runs?branch=...&head_sha=...`) and the
commit's check runs. Right before pushing, the test asks GitHub which
workflow runs it already lists on `scope/setup-check` for the commit; those
are an earlier test's and never count, so testing an unchanged main again
waits for its own runs. Only check runs filed under the counted workflow
runs' check suites count, so runs GitHub started for the same commit on main
do not. The test's push job answers it, so a push of an earlier test that
finishes late changes nothing. A test belongs to the GitHub repository it
pushed to: settings stop showing it once the repository is connected to
another one, and it does not hold up a test of the new one. Deleting the
branch afterwards retries like any push job. The
test ends when every workflow run it saw completed, or after 15 minutes, and
then deletes the branch. The check names it saw are listed, and each can be
made a required check with one click. A test that ends without any workflow
run says: "No workflows started. Check that your workflows include the
scope/** push trigger."

### Required checks

Maintainers list the check names GitHub must pass under Required checks in
the CI section of repository settings. They are the names GitHub's own branch
protection uses, such as `ci / test`.
`PUT /v1/repos/{owner}/{repo}/github/required-checks` replaces the list.
Every push to a request records an evaluation with one GitHub check per
required name and a tested commit. Changing the list affects heads pushed
afterwards.

A private request's tested commit is its head. A public contribution's head is
public history and lacks the private files, so GitHub tests a check commit
instead: the contribution merged onto private main with the same
`git merge-tree --write-tree` routine the merge uses. Its merge base is the
newest public main commit the head contains, which is current public main once
the head contains it, as the merge requires. Its parents are private main and
the head, its author and committer lines are the head's, and its message is
`Scope check for <request id> at <short head>`, so the same head and private
main always give the same commit. The evaluation records the private main and
merge base it used, and the push builds the commit again from them. A
contribution that conflicts with private code records a configuration error,
"This contribution conflicts with private code, so its checks cannot run. A
maintainer must resolve the conflict.", and nothing is pushed.

A check commit counts only while private main is the one it was built on,
because the merge applies the contribution to private main as it is. Once
private main moves, by a push or a merge, its checks are pending again, even
if they passed. Scope builds a new check commit on the new private main for
every open public contribution with started checks and pushes it at once: the
head is the one a maintainer approved, and private main is trusted. The merge
gate does the same for the request it is asked to merge, so a merge waits for
the new commit's checks. Results for the old commit no longer count. When the
contribution conflicts with the new private main, its checks become the
conflict configuration error.

The check commit is private code. It exists only in temporary private staging
repositories and on the connected GitHub repository. Request views and the
checks response name only the head, for maintainers and contributors alike. A
connected repository that became public receives no check commit until a
maintainer confirms, the same as a private request.

A request can merge when the latest run of each required name on the tested
commit passed: success, neutral or skipped. A re-run on GitHub is a newer run
and decides. A name with no run yet is pending, and the request view lists it
as having no run. Runs on any other commit never count, so a rebase or amend
needs its own green runs. Merge and auto-merge both wait for this.

### Pushing revisions

Scope pushes the tested commit to `scope/requests/<request id>` in the
connected repository with `git push --force`, so a new revision replaces the
branch. The installation token reaches git as an `http.extraHeader` through
git's environment, never in its arguments or the URL. Merging, closing or
deleting the request deletes the branch from the GitHub repository it was
pushed to. Deleting the Scope repository queues the same deletions; they
name the GitHub repository and installation themselves, so they run after
the Scope repository is gone, while the installation exists.

A maintainer's push goes to GitHub at once, even when no check is required,
so workflows still run. Anyone else's push waits until a maintainer approves
the checks, because a pushed branch receives the repository's secrets.
Approval names the head the maintainer reviewed, and a newer head is
refused. When the request changes files under `.github/workflows/`, the
request view warns the maintainer before approving. With no required checks, a contributor's
push is never sent.

Pushes are jobs in `scope_github_pushes`, run by a background loop in the API
with leases. Each push is claimed on its own with a lease well past the push
timeout. A branch's pushes are ordered by a sequence that only grows; a newer
push replaces queued ones, and a branch is pushed by one process at a time.
Right before git runs, a push checks, by the database's clock, that its claim
still holds and that no newer push of its branch was queued; otherwise it
sends and records nothing. A commit is pushed only while the repository is
still connected to the GitHub repository and installation the push was
queued for. A failed push is tried again after 30
seconds, then 2, 10 and 30 minutes, and then gives up. The request view shows
whether the revision is waiting for approval, being sent, sent, or failed,
and shows maintainers the last error. A push to a repository whose link is
gone gives up at once.

### Reading results

GitHub's API is the source of results. A `check_run`, `check_suite` or
`workflow_run` delivery for a commit some request was evaluated against makes
Scope read `GET /repos/{owner}/{repo}/commits/{sha}/check-runs?filter=all`
and replace what it stored for that commit. Runs are stored per GitHub
repository, and only the connected repository's runs count, so a Scope
repository reconnected to another GitHub repository starts over. Every read
is numbered before Scope asks GitHub, and its answer is stored only when no
later read was stored first, so a slow, older answer cannot bring back a
stale result.

A background reconciler reads the commits of started evaluations of open
requests every two minutes while their checks are pending and every ten once
they settle, which covers dropped deliveries, including a failed re-run of a
check that had passed. Merging, by hand or automatically, reads the commit
again when what Scope stored is more than a minute old or a newer read is
still asking GitHub, and does not merge while GitHub cannot answer. The
reconciler keeps the two-minute pace while any open request testing a commit
is pending. New results refresh open request views and wake auto-merge.

When the link is disconnected or removed, evaluations that ask GitHub become
configuration errors: they never pass and never wait forever.

GitHub starts nothing, and reports nothing, for a branch none of whose
workflows has the push trigger. So when the tested commit has had no check run
at all for 10 minutes after its push succeeded, the request view (web and
`scope request checks`) says "No workflows started. Check that your workflows
include the scope/** push trigger."

## Runs page

A repository with a GitHub link lists GitHub Actions workflow runs on its Runs
page instead of Scope's own runs:
`GET /v1/repos/{owner}/{repo}/github/workflow-runs`, readable by repository
members, returns the 50 most recent with their workflow, branch, commit,
status and time, and a link to the repository's Actions page for the rest.
Every run links to GitHub, which keeps the logs; a run on
`scope/requests/<id>` also links to its request while the request exists.
Repositories without a link keep their native runs. When such a repository
has no runs and no workflows of its own, and the server has a GitHub App,
the Runs page says its runs come from GitHub Actions once GitHub is connected
and offers maintainers Connect GitHub. The workflow runs response says
whether the server has a GitHub App (`configured`). A connected repository
without runs yet links maintainers to Test connection in the CI settings.

Runs are stored in `scope_github_workflow_runs`. A `workflow_run` delivery for
a connected repository makes Scope read `GET /repos/{owner}/{repo}/actions/runs/{id}`
and store what GitHub answers. A stored run only moves forward: a later
attempt (a re-run on GitHub) replaces it, and within an attempt a read never
moves it back from completed or replaces a newer one, even when GitHub dates
both reads to the same second. The read is kept in
`scope_github_workflow_run_reads` before the delivery is acknowledged, so one
GitHub does not answer is read again every 30 seconds with the push job's
backoff before it is given up. Deliveries for repositories that are not
connected are ignored. Each
stored run sends a `GitHubWorkflowRunsChanged` repository event, which open
Runs pages use to refresh their list in place.

## Webhooks

`POST /v1/github/webhooks` checks `X-Hub-Signature-256` over the raw body
before reading it and answers 401 to a bad signature. `api/src/github/webhook.rs`
names every event Scope acts on; other events are acknowledged with 204.

Deliveries can arrive late or be redelivered, so an installation event is
confirmed with GitHub before it changes a link: the app asks whether the
installation still exists or is suspended and, for a removed repository,
whether the installation still reaches it. If GitHub says access is intact,
the event is ignored. Connecting and applying an installation event hold the
same installation lock while they ask GitHub, so a removal that lands during
a connect is either seen by the connect or finds the new link.

Check deliveries for repositories or commits Scope does not watch (no request
evaluation and no running connection test uses them) are acknowledged with
204, and a failed read is left to the reconciler.

## Local development

`./dev/scope-dev up` passes the six `SCOPE_GITHUB_` variables from the root
`.env.local` to the API. Write the private key on one line with its line
breaks as `\n`. Use a separate development GitHub App.

To connect from another device on a tailnet, serve the web app and API with
`tailscale serve` and set `SCOPE_DEV_TAILNET_WEB_ORIGIN` and
`SCOPE_DEV_TAILNET_API_ORIGIN` in `.env.local`, as described in
`./dev/scope-dev help`. Register both `http://localhost:3000/github/setup` and
`<tailnet web origin>/github/setup` as callback URLs, and use one of them as
the Setup URL. GitHub must reach the webhook, so expose only that path
publicly with Tailscale Funnel and use the funnel address as the Webhook URL:

```sh
tailscale funnel --bg --https=8443 --set-path=/v1/github/webhooks \
  http://127.0.0.1:8080/v1/github/webhooks
```

## Scope's own repository

Scope's workflows in `.github/workflows` carry the `scope/**` push trigger.
The same change replaces the checks image with the runner base image, so the
switch has steps on both sides of the merge.

Before merging, and before any release that includes the change, follow
[Move publishing to the runner base image](../deploy/aws/OPERATIONS.md#move-publishing-to-the-runner-base-image).
It updates the AWS stacks and sets `SCOPE_RUNNER_IMAGE_AWS_ROLE_ARN`. The
first release afterwards publishes the runner base image, which needs both.

After that release:

1. Connect the Scope repository to `scope-vcs/scope-vcs` from its repository
   settings. That GitHub repository is public, so private requests and
   private files pushed there become public, as
   [Public GitHub repositories](#public-github-repositories) describes. The
   connect must be done by an owner who can change file visibility, and they
   must confirm the public-repository acknowledgment on the setup page.
2. Confirm that workflows start: push a request and check that CI runs on its
   `scope/requests/<id>` branch. Note the check names GitHub reports.
3. Choose the required checks in the Checks section. Expect `Required PR
   checks`, the job that aggregates every other CI result.
4. Delete what the old checks image left behind, as the last step of the
   operations guide describes: the retained ECR repository
   `scope-vcs/production/checks`, the GitHub variable
   `SCOPE_CHECKS_IMAGE_AWS_ROLE_ARN` and the `checks` container package on GHCR.
