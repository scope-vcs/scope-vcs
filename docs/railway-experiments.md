# Railway experiments

Production and staging are the two persistent Railway environments. Their exact provider IDs live in `.github/deployment-services.json`.

Before creating an experiment, choose an owner, a purpose, and a UTC expiry. Normally set the expiry 48 hours after creation. Use an explicit expiry that fits the experiment; the audit honors that timestamp.

Name the environment `test-<purpose>-<yyyymmdd>`, such as `test-pack-index-20260909`. Register its exact Railway environment ID in `.github/railway-experiments.json` when creating it:

```json
{
  "00000000-0000-0000-0000-000000000001": {
    "owner": "github-username",
    "expiresAt": "2026-09-11T18:00:00Z"
  }
}
```

Replace the example ID with the provider's actual environment ID. Merge the registration into main. Neither persistent environment may appear in this registry.

The independent **Audit Railway experiments** GitHub Actions workflow runs hourly. To check immediately, open its Actions page and choose **Run workflow** on main. Missing registration, invalid names, missing owners, expired timestamps, and registrations for environments that no longer exist fail the audit.

A failed audit does not stop services or delete data. The owner reviews the environment, archives data that must be kept, and then deletes the environment deliberately. Remove its registry entry after retirement. If more time is needed, review and update the explicit expiry before it passes.
