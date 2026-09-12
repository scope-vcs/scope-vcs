# Licensing

Scope's own code is licensed under Apache-2.0. The complete license is in
`LICENSE`; the copyright attribution is in `NOTICE`. The workspace manifest,
standalone CLI manifest, self-contained domain and API-contract manifests, and
web package metadata identify the same license.

Third-party code, fonts, and other material retain their original licenses.
Scope's Apache license does not replace those terms. Apache-2.0 does not grant
permission to use Scope's trademarks except as described in section 6.

Pagent is separately licensed under Apache-2.0 by its upstream project. The
original bundled archive omits its license text and package license field.
The inventory preserves the upstream grant from commit
`d0e8ba60e39e058d189d7dd121740afc30f32900`, whose application code is unchanged
from the bundled revision, and includes that grant in the web notices.
The supplement is bound to the exact archive checksum. A replacement archive
requires review and should include the upstream license itself.

## Dependency inventory

`legal/dependency-inventory.json` records the audited package versions, archive
checksums, license declarations, and the provenance of license and notice texts.
Each dependency occupies one JSON record line. Generated notices print repeated
MIT and Apache terms once and reference them from each applicable document.
Sharing requires identical words and punctuation; differences in whitespace are
ignored. Copyright notices, additional terms, and distinct wording are retained.
The inventory covers both Rust lockfiles, the web lockfile, and the dependency
analyzer npm lockfile, including platform-specific and development dependencies.
This is a conservative set;
listing a package does not mean every distribution contains it.

The audit also includes copied shadcn UI source, which is not a package-lockfile
dependency. Its MIT attribution must remain with the web application. IBM Plex
Sans and Commit Mono retain their OFL notices; Lucide retains its ISC notices.

For dual-licensed packages, the inventory identifies the selected license while
retaining supplied notices. DOMPurify uses its Apache-2.0 option. Lightning CSS
build tools retain MPL-2.0 terms, and caniuse-lite data retains CC-BY-4.0 terms.
Where an archive declares a standard license but omits its full text, the audit
records the declaration and separately sourced standard terms. It does not
invent missing copyright names or years. Additional Node.js attribution is
included for code copied into the wasm-util dependency.

The inventory concerns application dependencies and copied application source.
Base operating-system images and separately installed tools retain the license
material supplied by their distributors.

## Regeneration and checks

Use Python 3.11 or newer. Install the pinned regeneration dependency into your
Python environment, then regenerate from the locked archives:

```sh
python -m pip install -r dev/licensing/requirements.txt
python dev/licensing/generate.py
```

Regeneration requires network access. The generator verifies downloaded archives
against lockfile checksums and records upstream evidence for license text omitted
from a package archive. Review dependency or attribution changes together with
the regenerated inventory and texts.

The normal repository policy check verifies the committed results offline:

```sh
python dev/licensing/generate.py --check
```

This check also runs through `./dev/check guardrails`. It detects stale lockfile
inputs, changed generated notices, and web copies that differ from the root
license and notice. It is a freshness check, not a vulnerability scanner.

## Distribution

| Distribution | License material |
| --- | --- |
| Repository source | Root `LICENSE`, `NOTICE`, inventory, and generated third-party notices |
| CLI binary | `scope licenses` prints embedded Apache, Scope, and Rust dependency texts without authentication or a repository; `--json` returns the same material as JSON |
| Web application | Public `/licenses` links to `/LICENSE.txt`, `/NOTICE.txt`, and `/third-party-licenses.txt`; the build includes these assets |
| Backend release archive | `LICENSE`, `NOTICE`, and `third-party-rust.txt` accompany the binaries |
| Backend and CLI service deployments | License files accompany the deployed binaries in `bin/` |
| Dependency worker image | Analyzer notices reside beside the analyzer in `/app/dependency-analyzer/` |
| Scope runner image | Application license files reside in `/scope/licenses/` |

The web copies of `LICENSE` and `NOTICE` are generated from the root files. Edit
the root files and regenerate instead of editing copies. Changes to shared
license material select all affected build and deployment lanes.

Public Scope projections must include the applicable `LICENSE`, `NOTICE`, and
third-party notices alongside the files they expose. This checkout has no Scope
remote or local Scope repository visibility configuration, so it cannot establish
the contents of a deployed Scope projection. Check those paths in the published
projection when this repository is connected to Scope. The public GitHub
repository publishes the complete committed tree.
