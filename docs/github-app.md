# Scope GitHub App

A maintainer connects a Scope repository to the project's GitHub repository
by installing the Scope GitHub App. Later phases push request revisions to
that repository and read the results of its workflows. One app serves every
Scope repository on a server.

## Registering the app

Create a GitHub App owned by the organization that runs Scope, with:

- **Callback URL**: `https://<web origin>/github/setup`. Also set the
  **Setup URL** to the same address if GitHub offers it.
- **Request user authorization (OAuth) during installation**: on. GitHub then
  sends the installer back with a code that proves which installations and
  repositories they can reach. **Redirect on update**: on, so changing the
  repositories of an existing installation also returns to Scope.
- **Webhook URL**: `https://<api origin>/v1/github/webhooks`, with a random
  webhook secret.
- **Repository permissions**: Contents read and write, Workflows read and
  write, Checks read, Actions read, Metadata read.
- **Subscribed events**: Check run, Workflow run, Installation, Installation
  repositories. Installation events are always delivered to GitHub Apps; the
  other three are chosen on the registration page.
- **Where can this app be installed**: any account.

Generate a private key and a client secret on the app's page.

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

1. A maintainer chooses Connect GitHub in repository settings. The API returns
   the app's install URL with a signed `state` naming the Scope repository,
   the maintainer, and a ten-minute expiry.
2. After GitHub's install screen, `/github/setup` sends the state, the
   installation ID and the OAuth code to `POST /v1/github/setup`. The API checks
   the state, that the same person is signed in and is still a maintainer,
   exchanges the code for a user token, and confirms through that token that
   the person can reach the installation. It returns the installation's
   repositories that person can reach and a signed grant listing them. The
   user token is not stored.
3. The maintainer picks a repository (it is chosen for them when there is only
   one) and `POST /v1/repos/{owner}/{repo}/github` stores the link after checking
   the grant and confirming with an installation token that the app still
   reaches the repository.

An installation ID arriving from a redirect is never trusted on its own:
installation IDs are guessable.

A Scope repository has at most one link, and a GitHub repository is connected
to at most one Scope repository at a time. Disconnecting in settings removes
the link. Uninstalling or suspending the app, or removing the repository from
the installation, keeps the link as disconnected with the reason, and
settings offer to reconnect. Unsuspending does not reconnect by itself.

## Webhooks

`POST /v1/github/webhooks` checks `X-Hub-Signature-256` over the raw body
before reading it and answers 401 to a bad signature. `api/src/github/webhook.rs`
names every event Scope acts on; other events are acknowledged with 204.
