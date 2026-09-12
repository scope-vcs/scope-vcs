use super::*;

const GALLERY_README: &str = r#"# Dependency warning gallery

This repository is a deterministic local-development fixture for dependency visibility analysis.
`/internal` is private and every other path is public.

Expected observations:

- The headline reports 7 affected public source files, backed by 8 distinct public-to-private source/target pairs, with no analysis gaps.
- `src/chain/a.ts` imports public `b.ts`, while `b.ts` imports private `c.ts`. Only the `b.ts` to `c.ts` edge is a warning. The duplicate relative and `@private/*` alias imports collapse to one source/target edge.
- Type-only imports, an unused namespace import, a re-export, a side-effect import, literal CommonJS `require`, and literal dynamic `import()` each cross from public source to private target.
- `src/type-consumer.ts` reaches two private type modules from one source, exercising a warning with multiple targets.
- The re-export case uses deliberately long source and target paths.
- The `node:path` built-in import is external and does not produce a repository edge.
"#;

const CLEAN_README: &str = r#"# Clean dependency graph

This local-development fixture has all safe visibility combinations and should report no warnings or analysis gaps:

- public source to public target
- private source to private target
- private source to public target

`/internal` is private and every other path is public.
"#;

const INCOMPLETE_README: &str = r#"# Incomplete dependency analysis

This local-development fixture keeps one known public-to-private warning while also exercising incomplete analysis:

- `src/known-warning.ts` imports `internal/private.ts`.
- `src/missing.ts` names a literal module that is not in the repository.
- `src/runtime-dynamic.ts` uses nonliteral dynamic `import()` and `require()` calls.
- `src/malformed.ts` is intentionally malformed source.

The useful result is partial: one affected public source and its warning pair remain visible alongside 4 gap records from the three incomplete source files (the runtime-dynamic file contributes two).
"#;

const UNSUPPORTED_README: &str = r#"# Unsupported source gallery

This local-development fixture contains Rust source only. Dependency analysis should finish without warnings or JavaScript/TypeScript analysis gaps and identify the two `.rs` source files as unsupported.

`/internal` is private and every other path is public.
"#;

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@private/*": ["internal/*"]
    }
  }
}
"#;

#[derive(Clone, Copy)]
pub(super) struct SeedFile {
    pub(super) path: &'static str,
    pub(super) content: &'static str,
}

pub(super) struct SeedRepository {
    pub(super) name: &'static str,
    pub(super) files: &'static [SeedFile],
}

impl SeedFile {
    const fn new(path: &'static str, content: &'static str) -> Self {
        Self { path, content }
    }

    fn scope_path(self) -> String {
        format!("/{}", self.path)
    }

    fn visibility(self) -> Visibility {
        if self.path.starts_with("internal/") {
            Visibility::Private
        } else {
            Visibility::Public
        }
    }
}

const GALLERY_FILES: &[SeedFile] = &[
    SeedFile::new("README.md", GALLERY_README),
    SeedFile::new("tsconfig.json", TSCONFIG),
    SeedFile::new(
        "src/chain/a.ts",
        "import { value } from './b'\nexport const result = value\n",
    ),
    SeedFile::new(
        "src/chain/b.ts",
        "import { secret } from '../../internal/chain/c'\nimport { secret as aliasedSecret } from '@private/chain/c'\nexport const value = secret + aliasedSecret\n",
    ),
    SeedFile::new("internal/chain/c.ts", "export const secret = 21\n"),
    SeedFile::new(
        "src/type-consumer.ts",
        "import type { Token } from '../internal/types/token'\nimport type { Profile } from '../internal/types/profile'\nexport type Session = { token: Token; profile: Profile }\n",
    ),
    SeedFile::new("internal/types/token.ts", "export type Token = string\n"),
    SeedFile::new(
        "internal/types/profile.ts",
        "export interface Profile { name: string }\n",
    ),
    SeedFile::new(
        "src/namespace.ts",
        "import * as privateFlags from '../internal/namespace-secret'\nexport const visible = true\n",
    ),
    SeedFile::new(
        "internal/namespace-secret.ts",
        "export const privateFlag = true\n",
    ),
    SeedFile::new(
        "src/features/with-a-deliberately-long-directory-name/exports/with-an-equally-long-public-filename.ts",
        "export { internalReleaseName } from '../../../../internal/features/with-a-deliberately-long-directory-name/releases/with-an-equally-long-private-filename'\n",
    ),
    SeedFile::new(
        "internal/features/with-a-deliberately-long-directory-name/releases/with-an-equally-long-private-filename.ts",
        "export const internalReleaseName = 'hidden'\n",
    ),
    SeedFile::new(
        "src/bootstrap.ts",
        "import '../internal/register-secret'\nexport const bootstrapped = true\n",
    ),
    SeedFile::new(
        "internal/register-secret.ts",
        "globalThis.__scopeFixtureRegistered = true\n",
    ),
    SeedFile::new(
        "src/legacy.cjs",
        "const secret = require('../internal/legacy-secret.cjs')\nmodule.exports = secret\n",
    ),
    SeedFile::new(
        "internal/legacy-secret.cjs",
        "module.exports = { value: 'private' }\n",
    ),
    SeedFile::new(
        "src/lazy.ts",
        "export async function loadSecret() {\n  return import('../internal/lazy-secret')\n}\n",
    ),
    SeedFile::new(
        "internal/lazy-secret.ts",
        "export const lazySecret = 'private'\n",
    ),
    SeedFile::new(
        "src/external.ts",
        "import path from 'node:path'\nexport const separator = path.sep\n",
    ),
];

const CLEAN_FILES: &[SeedFile] = &[
    SeedFile::new("README.md", CLEAN_README),
    SeedFile::new(
        "src/public-entry.ts",
        "import { publicValue } from './public-value'\nexport const result = publicValue + 1\n",
    ),
    SeedFile::new("src/public-value.ts", "export const publicValue = 1\n"),
    SeedFile::new(
        "internal/private-entry.ts",
        "import { privateValue } from './private-value'\nimport { publicValue } from '../src/public-value'\nexport const result = privateValue + publicValue\n",
    ),
    SeedFile::new(
        "internal/private-value.ts",
        "export const privateValue = 2\n",
    ),
];

const INCOMPLETE_FILES: &[SeedFile] = &[
    SeedFile::new("README.md", INCOMPLETE_README),
    SeedFile::new(
        "src/known-warning.ts",
        "import { privateValue } from '../internal/private'\nexport const exposed = privateValue\n",
    ),
    SeedFile::new(
        "internal/private.ts",
        "export const privateValue = 'private'\n",
    ),
    SeedFile::new(
        "src/missing.ts",
        "import { missing } from './does-not-exist'\nexport const value = missing\n",
    ),
    SeedFile::new(
        "src/runtime-dynamic.ts",
        "declare const moduleName: string\nexport const dynamicallyLoaded = import(moduleName)\nexport const requiredAtRuntime = require(moduleName)\n",
    ),
    SeedFile::new(
        "src/malformed.ts",
        "import { broken from './never-resolves'\nexport const nope = )\n",
    ),
];

const UNSUPPORTED_FILES: &[SeedFile] = &[
    SeedFile::new("README.md", UNSUPPORTED_README),
    SeedFile::new(
        "src/main.rs",
        "mod private;\nfn main() { println!(\"fixture\"); }\n",
    ),
    SeedFile::new(
        "internal/private.rs",
        "pub const SECRET: &str = \"private\";\n",
    ),
];

const DEPENDENCY_REPOSITORIES: &[SeedRepository] = &[
    SeedRepository {
        name: "dependency-gallery",
        files: GALLERY_FILES,
    },
    SeedRepository {
        name: "dependency-clean",
        files: CLEAN_FILES,
    },
    SeedRepository {
        name: "dependency-incomplete",
        files: INCOMPLETE_FILES,
    },
    SeedRepository {
        name: "dependency-unsupported",
        files: UNSUPPORTED_FILES,
    },
];

pub(super) fn seed_dependency_repositories(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
) -> Result<Vec<(Repository, GitSegmentUpload)>, ApiError> {
    DEPENDENCY_REPOSITORIES
        .iter()
        .map(|fixture| {
            dependency_repository(
                object_store,
                git_segment_store,
                owner,
                fixture.name,
                fixture.files,
            )
        })
        .collect()
}

fn dependency_repository(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
    name: &str,
    files: &[SeedFile],
) -> Result<(Repository, GitSegmentUpload), ApiError> {
    let mut repository = repo(owner, name, Visibility::Public)?;
    repository
        .policy
        .add_rule(VisibilityRule::private(
            ScopePath::parse("/internal").map_err(ApiError::internal)?,
        ))
        .map_err(ApiError::internal)?;
    repository.repo_config.visibility.rules.push(
        scope_domain::repo_config::RepoConfigVisibilityRule {
            path: "/internal/**".into(),
            visibility: scope_domain::repo_config::ConfigVisibility::Private,
        },
    );

    let changes = files
        .iter()
        .map(|file| {
            add_change(
                &file.scope_path(),
                blob(object_store, file.content)?,
                file.visibility(),
            )
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    repository.graph.commits.push(commit(
        &repository,
        &format!("dev-{name}-1"),
        &format!("Seed {name}"),
        changes,
    ));
    populate_seed_live_files(&mut repository);
    repository.record.lifecycle_state = RepoLifecycleState::Ready;

    let git_files = files
        .iter()
        .map(|file| (file.path, file.content))
        .collect::<Vec<_>>();
    let (head, pack_span, upload) = git_pack_state(
        git_segment_store,
        &repository.record.id,
        name,
        &[SeedGitCommit {
            files: &git_files,
            message: &format!("Seed {name}"),
        }],
    )?;
    repository.git_head = Some(head);
    repository.git_pack_spans.push(pack_span);
    Ok((repository, upload))
}

#[cfg(test)]
pub(super) fn fixtures() -> &'static [SeedRepository] {
    DEPENDENCY_REPOSITORIES
}
