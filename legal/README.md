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

`git-sources.json` records the reviewed immutable Cargo Git sources accepted by
`dev/licensing/generate.py`: exact source and commit, archive URL and SHA-256,
workspace manifest, and each crate/version's manifest path. Git source archives
are independently verified; registry package metadata does not stand in for
Git crate licensing. The collector checks actual crate identity and version,
resolves inherited workspace licensing metadata, and includes workspace and
crate license files. An unreviewed commit, crate/version, changed archive, or
mismatched manifest fails collection. Review this record when changing a Git
pin, then run `python3 dev/licensing/generate.py` and
`python3 dev/licensing/generate.py --check` after updating the lockfile.
