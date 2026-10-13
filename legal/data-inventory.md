# Personal data inventory

This inventory is the maintainer-facing source for the
[privacy policy](privacy-policy.md). Update both together when Scope starts
collecting new personal data, adds a service provider, or changes a retention
period.

## Data

| Data | Stored in | Retention | Deleted by |
| --- | --- | --- | --- |
| Handle, verified email, Clerk subject | `scope_users`, `scope_auth_identities`; Clerk | Until account deletion | Account deletion use case; Clerk user deletion effect |
| CLI session token hashes and labels | `scope_cli_sessions` and CLI login tables | Until account deletion or session expiry | Cascades with the user |
| Repository contents, including commit author names and emails | Postgres and repository object storage on Railway | Until repository deletion | Repository deletion and the object cleanup queue |
| Requests, revisions, discussions, replies, ratings | Postgres | Until repository deletion; authored work in other people's repositories remains with a null author after account deletion | Repository deletion; account deletion |
| Media attachments | Railway media bucket `scope-request-media` (US East) | Until repository deletion | Media cleanup |
| Repository invitations, invited email | `scope_repository_invites`, invite email delivery rows; sent through Resend | 30 days after acceptance, revocation or expiry | Invite retention pass |
| Workflow runs and logs | Postgres, repository object storage; executed on AWS runners | 30 days after the run finishes | `api/src/run_retention.rs` |
| Database recovery sets | AWS S3 `scope-recovery-*`, SSE-S3 encrypted | 42 days | S3 lifecycle in `deploy/aws/recovery/storage.yaml` |
| Product analytics events | PostHog US project, through the web `/e` proxy and backend transport | PostHog project retention | PostHog; events are keyed by opaque `scope_usr_…` IDs that become unlinkable once the user row is deleted |
| Server logs: request method, path, timing, errors; Railway HTTP logs also record client IP addresses | Railway logs | Railway log retention | Railway |
| Application logs and request traces: service log lines, which can include repository names, file paths and error details; spans with internal IDs, routes and timing | PostHog US project, sent by each service over OpenTelemetry | PostHog log and trace retention, 14 days by default | PostHog |

The analytics collection contract, including excluded fields, is in
[docs/product-analytics.md](../docs/product-analytics.md). What services export
as logs and traces is in [docs/observability.md](../docs/observability.md).

## Browser storage

| Key | Purpose | Consent needed |
| --- | --- | --- |
| Clerk cookies | Sign-in | No, strictly necessary |
| `scope-theme` in `localStorage` | Theme the user chose | No, user-requested |
| `scope-requests-sidebar` cookie, one year | Whether the user collapsed the requests sidebar, read during server rendering | No, user-requested |
| Attachment drafts in `sessionStorage` | Unsent attachments in the current tab | No, user-requested |
| `web/src/lib/home-flash.ts` values in `sessionStorage` | One-time notices shown after a redirect, cleared on read | No, strictly necessary |

Analytics IDs live in page memory only. Adding any analytics or tracking storage
requires a consent mechanism first.

## Service providers

| Provider | Purpose | Region |
| --- | --- | --- |
| Clerk | Authentication | US |
| Railway | Hosting, Postgres, object storage, logs | US |
| Amazon Web Services | Workflow runners, recovery backups, audit logs | us-east-1 |
| PostHog | Product analytics, application logs and traces | US |
| Resend | Invitation email | US |

Vercel hosts DNS for scopevcs.com and receives no user data.
