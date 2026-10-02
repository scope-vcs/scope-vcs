# Production GitHub App

A checklist for registering the production Scope GitHub App and turning it on
for the production API. [GitHub App](github-app.md) explains each setting.
The origins below come from `.github/deployment-services.json`
(`releaseAvailability.production`).

## Register the app

Create the app under the `scope-vcs` organization (Organization settings,
Developer settings, GitHub Apps, New GitHub App):

- [ ] **GitHub App name**: TODO: choose the public name, such as "Scope". It
      sets the slug in `github.com/apps/<slug>`.
- [ ] **Homepage URL**: `https://scopevcs.com`
- [ ] **Callback URL**: `https://scopevcs.com/github/setup`
- [ ] **Expire user authorization tokens**: leave GitHub's default.
- [ ] **Request user authorization (OAuth) during installation**: off.
- [ ] **Setup URL**: `https://scopevcs.com/github/setup`, with **Redirect on
      update** on.
- [ ] **Webhook**: active, URL `https://api.scopevcs.com/v1/github/webhooks`,
      with a secret generated for it, for example `openssl rand -hex 32`. Keep
      the secret for the API variables below.
- [ ] **Repository permissions**: Contents read and write, Workflows read and
      write, Checks read-only, Actions read-only, Metadata read-only.
- [ ] **Subscribe to events**: Check run, Check suite, Workflow run,
      Repository. Installation events arrive without subscribing.
- [ ] **Where can this GitHub App be installed**: Any account.

After creating it:

- [ ] Generate a private key and download the PEM file.
- [ ] Generate a client secret.
- [ ] Note the App ID, Client ID and slug from the app's page.

## Configure the API

Set all six variables on the `scope-api` service in Railway's production
environment, then redeploy it. Setting only some of them stops the API at
startup.

- [ ] `SCOPE_GITHUB_APP_ID`
- [ ] `SCOPE_GITHUB_APP_SLUG`
- [ ] `SCOPE_GITHUB_APP_PRIVATE_KEY`, the PEM file with its line breaks
      written as `\n`
- [ ] `SCOPE_GITHUB_APP_CLIENT_ID`
- [ ] `SCOPE_GITHUB_APP_CLIENT_SECRET`
- [ ] `SCOPE_GITHUB_WEBHOOK_SECRET`

Check that the CI section of a repository's settings no longer says GitHub is
not configured on this server, and that the app's Advanced page on GitHub
shows deliveries answered with 204.

## Connect scope-vcs/scope-vcs

- [ ] Install the app on `scope-vcs/scope-vcs` from the connect page in the
      Scope repository's CI settings.
- [ ] `scope-vcs/scope-vcs` is public, so connecting asks you to confirm that
      everything Scope pushes there, private requests and private files
      included, becomes public on GitHub. Only a maintainer who can change
      file visibility can confirm.
- [ ] Choose how many recent workflow runs to import before connecting.
- [ ] Add the `scope/**` push trigger to `.github/workflows/ci.yml`, which
      runs on `pull_request` only. Its jobs read the pull request (the
      concurrency group and checkout), so they need a fallback for a branch
      push. Then use Test connection.
- [ ] Under Required checks, require `Required PR checks`, the CI job that
      main's branch protection requires (`deploy/automation/main-protection.json`).
