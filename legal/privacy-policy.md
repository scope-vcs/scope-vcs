# Privacy policy

Effective September 24, 2026

Scope is a version control service at scopevcs.com, run by Adam Blumoff as an
individual. This policy explains what personal data Scope handles, why, who
processes it, how long it is kept, and what you can do about it. Questions and
requests go to [hello@scopevcs.com](mailto:hello@scopevcs.com).

## What Scope collects

**Account data.** When you sign up, Clerk handles sign-in and passes Scope your
verified email address and a Clerk account identifier. Scope stores your
handle, email address and that identifier. If you sign in with another provider
through Clerk, Clerk receives the profile data that provider shares.

**Content you add.** Repositories you push, including commit author names and
email addresses recorded in Git history; requests, revisions, discussions and
replies; media attachments; ratings; and repository invitations, including the
invited email address.

**CLI sessions.** When you sign in from the Scope CLI, Scope stores a hashed
session token, a session label, and when the session was created, last used,
expires and was revoked.

**Workflow runs.** Logs and results from workflows that run on your
repositories.

**Product analytics.** The website records page views, error types and
performance measurements. The API records outcomes such as pushes, submitted
requests and completed workflow runs. Events are keyed by a random internal ID,
never by your name or email. They exclude names, email addresses, repository
names, source code, file paths, request titles and discussion content. The
website keeps its analytics ID in page memory only. Nothing is written to
cookies or browser storage for analytics. Browser analytics stops completely if
your browser sends Do Not Track or Global Privacy Control.

**Operational logs.** Scope's servers log request paths, timing and errors to
operate and secure the service. Railway, Scope's hosting provider, also records
your IP address in its HTTP logs. The analytics proxy removes your IP address
before events are sent to PostHog.

Scope does not sell or share personal data for advertising, and does not use
advertising cookies. The website stores only what it needs to work: Clerk's
sign-in cookies, your theme preference, and unsent attachment drafts in the
current tab.

## Why Scope uses it

- To provide the service you signed up for: hosting repositories, running
  workflows, and sending the invitations you ask Scope to send. For users in
  the EU and UK, the legal basis is performance of a contract.
- To keep the service secure and working, and to understand which features are
  used so they can be improved. The legal basis is legitimate interest. You can
  opt out of browser analytics with Do Not Track or Global Privacy Control.
- To meet legal obligations, such as responding to valid legal requests.

## Who processes it

Scope uses these service providers. Each receives only what it needs for its
job.

| Provider | Purpose | Location |
| --- | --- | --- |
| Clerk | Sign-in and account identity | United States |
| Railway | Application hosting, database and file storage | United States |
| Amazon Web Services | Workflow runners and encrypted database backups | United States |
| PostHog | Product analytics | United States |
| Resend | Sending invitation emails | United States |

Scope stores data in the United States. If you use Scope from elsewhere, your
data is transferred to the United States.

Repository content is visible to the people you give access to, and public
repositories are visible to everyone.

## How long it is kept

| Data | Retention |
| --- | --- |
| Account data | Until you delete your account |
| Repositories and their requests, discussions and attachments | Until the repository is deleted |
| Workflow runs and logs | 30 days after the run finishes |
| Repository invitations, including the invited email | 30 days after the invitation is accepted, revoked or expires |
| Database backups | 42 days |
| Server and hosting logs, including IP addresses | Railway's log retention period |
| Analytics events | Deleted on the retention schedule of Scope's PostHog project |

## Deleting your account

You can delete your account from your account page. Scope then deletes your
account data, CLI sessions and the repositories you own. Deleted data leaves
backups within 42 days.

If you own a repository that other people are members of, delete it before
deleting your account. Work you contributed to other people's repositories,
such as requests and discussion replies, stays in those repositories and shows
"Deleted user".

Git history is different. Commit author names and email addresses are part of
each commit, so they remain in other people's repositories and in any clones
made before deletion. Scope cannot remove them from copies it doesn't control.

## Your rights

Depending on where you live, you can ask to access, correct, delete or export
your personal data, object to or restrict its processing, and withdraw consent
where processing relies on it. Email
[hello@scopevcs.com](mailto:hello@scopevcs.com) from the address on your account
and Scope will respond within 30 days. You can also export your repositories at
any time by cloning them.

California residents have the right to know, delete and correct personal
information, and to opt out of its sale or sharing. Scope does not sell or share
personal information. Scope will not discriminate against you for exercising
these rights.

If you are in the EU or UK and believe Scope has mishandled your data, you can
complain to your local data protection authority.

## Children

Scope is not for children under 13. You must be at least 13, or older where
your country's law requires, to use it. If Scope learns that it holds an
account for a child under 13, the account is deleted.

## Security

Connections to Scope use TLS. Session tokens are stored hashed, and backups are
encrypted. To report a security problem, see [the security
policy](https://github.com/scope-vcs/scope-vcs/security/policy).

## Changes

When this policy changes, the effective date above changes. For significant
changes, Scope will notify account holders by email or in the product before the
change takes effect.
