# Legal

This directory is the single source for Scope's legal material.

| Source | Published at |
| --- | --- |
| `privacy-policy.md` | `/privacy`, imported by the web build |
| `terms-of-service.md` | `/terms`, imported by the web build |
| `SECURITY.md` | `.github/SECURITY.md`, GitHub's security policy |
| `security.txt` | `web/public/.well-known/security.txt` |
| `data-inventory.md` | Maintainer reference for the privacy policy |

Edit the sources here, then run `python3 dev/legal/distribute.py` to refresh
the copies. `./dev/check guardrails` fails when a copy is stale or when
`security.txt` has expired. Update the policies' effective dates when their
meaning changes.

Licensing material, including the dependency inventory and third-party notices,
is described in [docs/licensing.md](../docs/licensing.md).
