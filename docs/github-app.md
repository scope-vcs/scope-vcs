# Scope GitHub App

A maintainer connects a Scope repository to the project's GitHub repository
by installing the Scope GitHub App. Later phases push request revisions to
that repository and read the results of its workflows. One app serves every
Scope repository on a server.

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
- **Subscribed events**: Check run, Workflow run, Installation, Installation
  repositories. Installation events are always delivered to GitHub Apps; the
  other three are chosen on the registration page.
- **Where can this app be installed**: any account.

Generate a private key and a client secret on the app's page. The API also
needs `SCOPE_APP_ORIGIN` set to the web origin, which it uses to build the
callback address.

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

1. A maintainer chooses Connect GitHub in repository settings.
   `POST /v1/repos/{owner}/{repo}/github/authorize` returns GitHub's OAuth URL
   for the app with a signed `state` naming the Scope repository, the
   maintainer, and a ten-minute expiry.
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

## Webhooks

`POST /v1/github/webhooks` checks `X-Hub-Signature-256` over the raw body
before reading it and answers 401 to a bad signature. `api/src/github/webhook.rs`
names every event Scope acts on; other events are acknowledged with 204.
