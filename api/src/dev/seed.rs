#[path = "seed/git_fixtures.rs"]
mod git_fixtures;
use git_fixtures::*;
#[cfg(any(test, feature = "local-dev"))]
#[path = "seed/dependency_repositories.rs"]
mod dependency_repositories;
#[path = "seed/git_segments.rs"]
mod git_segments;
#[path = "seed/request_discussions.rs"]
mod request_discussions;
#[path = "seed/request_revisions.rs"]
mod request_revisions;
#[cfg(any(test, feature = "local-dev"))]
#[path = "seed/runs.rs"]
mod runs;
#[cfg(test)]
#[path = "seed/tests.rs"]
mod tests;
#[path = "seed/workflow_files.rs"]
mod workflow_files;
use git_segments::store_seed_git_pack;
#[cfg(test)]
pub(super) use git_segments::test_seed_git_segment_store;
#[cfg(any(feature = "local-dev", feature = "smoke-seed"))]
pub(crate) use request_discussions::seed_request_discussion_gallery;
use request_revisions::SeedRequestRevision;
#[cfg(feature = "local-dev")]
pub(crate) use runs::seed_run_gallery;
use scope_git::DEFAULT_GIT_BRANCH;
use workflow_files::{PUBLIC_DEMO_CHECKS_WORKFLOW, PUBLIC_DEMO_LINT_WORKFLOW};

use crate::error::ApiError;
use scope_domain::{
    account::UserAccount,
    content::SourceBlob,
    policy::{ScopePath, Visibility, VisibilityRule},
    projection::LogicalCommitOrigin,
    projection::{FileChange, LogicalCommit},
    repository::git::{GitHead, GitPackSpan, GitSegmentUpload},
    repository::{RepoLifecycleState, Repository},
    requests::{
        EditRequestIdentityInput, RecordRequestRevisionInput, RecordWorkingRequestUploadInput,
        RequestActorRole, RequestAudience, StartRequestFacts, StartRequestInput,
        canonical_request_ref, edit_request_identity, record_request_revision,
        record_working_request_upload, start_request,
    },
};
use scope_object_store::{ContentObjectKind, ObjectStore, put_content_object, put_source_blob};
use std::{
    fs,
    path::Path as FsPath,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

static SEED_TEMP_REPO_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DevSeedUser {
    pub(crate) email: String,
    pub(crate) handle: String,
}

pub(crate) const DEV_SEED_USER_ID: &str = "scope_usr_dev_seed";
const PUBLIC_DEMO_README_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Scope public demo</title>
  <style>
    :root {
      color-scheme: light dark;
      font-family: Inter, ui-sans-serif, system-ui, sans-serif;
      color: #17201d;
      background: #f4f1e9;
    }

    * { box-sizing: border-box; }

    body {
      margin: 0;
      min-height: 100vh;
      background:
        radial-gradient(circle at top right, rgba(76, 115, 93, .2), transparent 42rem),
        #f4f1e9;
    }

    main {
      width: min(100% - 2rem, 62rem);
      margin-inline: auto;
      padding: clamp(4rem, 10vw, 8rem) 0;
    }

    .eyebrow {
      margin: 0 0 1rem;
      color: #4c735d;
      font-size: .75rem;
      font-weight: 700;
      letter-spacing: .16em;
      text-transform: uppercase;
    }

    h1 {
      max-width: 12ch;
      margin: 0;
      font-family: Georgia, serif;
      font-size: clamp(3.25rem, 10vw, 7.5rem);
      font-weight: 500;
      letter-spacing: -.055em;
      line-height: .9;
    }

    .intro {
      max-width: 42rem;
      margin: 2rem 0 0;
      color: #4d5a55;
      font-size: clamp(1rem, 2vw, 1.25rem);
      line-height: 1.7;
    }

    .details {
      display: grid;
      grid-template-columns: repeat(3, 1fr);
      gap: 1.5rem;
      margin-top: clamp(4rem, 9vw, 7rem);
      padding-top: 1.5rem;
      border-top: 1px solid rgba(23, 32, 29, .22);
    }

    .details p { margin: .45rem 0 0; line-height: 1.5; }
    .details strong { font-size: .8rem; letter-spacing: .04em; text-transform: uppercase; }
    code { font: .9em ui-monospace, SFMono-Regular, Consolas, monospace; }

    @media (max-width: 40rem) {
      .details { grid-template-columns: 1fr; }
    }

    @media (prefers-color-scheme: dark) {
      :root { color: #e7ece8; background: #111614; }
      body {
        background:
          radial-gradient(circle at top right, rgba(116, 170, 139, .16), transparent 36rem),
          #111614;
      }
      .eyebrow { color: #8fc5a6; }
      .intro { color: #aebbb4; }
      .details { border-color: rgba(231, 236, 232, .2); }
    }
  </style>
</head>
<body>
  <main>
    <p class="eyebrow">Scope public demo</p>
    <h1>Public by design.</h1>
    <p class="intro">
      A repository can publish a clear, expressive front door without exposing the work that
      belongs behind it. This page is the committed <code>README.html</code>, rendered as-is.
    </p>
    <section class="details" aria-label="Repository details">
      <div>
        <strong>Visible</strong>
        <p>The homepage and a tiny TypeScript example.</p>
      </div>
      <div>
        <strong>Private</strong>
        <p>Internal planning stays out of the public projection.</p>
      </div>
      <div>
        <strong>Portable</strong>
        <p>No build, scripts, SDK, or remote assets required.</p>
      </div>
    </section>
  </main>
</body>
</html>
"#;
const PUBLIC_DEMO_APP: &str =
    "export function greet(name: string) {\n  return `hello ${name}`\n}\n";
const PUBLIC_DEMO_PLAN: &str =
    "# Internal Plan\n\nPrivate content stays out of public projections.\n";
const UPDATE_DEMO_INITIAL_README: &str = "# Update Demo\n\nThis repository has a clean published baseline.\n\n[Read the release guide](docs/release.md).\n";
const UPDATE_DEMO_RULES: &str = "";
const UPDATE_DEMO_RELEASE_GUIDE: &str =
    "# Release flow\n\nDocument the release before publishing the next version.\n";
const UPDATE_DEMO_INTERNAL_NOTES: &str =
    "# Internal notes\n\nOnly repository maintainers can read this file.\n";
const UPDATE_DEMO_RETRY_HELPER: &str =
    "export function retryDelay(attempt: number) {\n  return Math.min(attempt * 250, 2000)\n}\n";
const UPDATE_DEMO_TROUBLESHOOTING: &str =
    "# Troubleshooting\n\nExplain how to recover when the remote is unavailable.\n";
const UPDATE_DEMO_CLI_EXPERIMENT: &str =
    "experimental output: checking repository state before push\n";
const UPDATE_DEMO_QUEUE_DRAFT: &str =
    "# Request queue copy\n\nTighten the language before asking for review.\n";
const UPDATE_DEMO_CACHE_NOTE: &str =
    "# Cache note\n\nRecord the tradeoff without changing repository behavior.\n";

pub(crate) fn catalog(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    seed_user: DevSeedUser,
) -> Result<scope_postgres::db::CatalogFixture, ApiError> {
    let owner = seed_user_account(seed_user);
    let [contributor, maintainer] = request_discussions::collaborators();
    let mut catalog = scope_postgres::db::CatalogFixture::default();
    catalog.users.insert(owner.id.clone(), owner.clone());
    catalog
        .users
        .insert(contributor.id.clone(), contributor.clone());
    catalog
        .users
        .insert(maintainer.id.clone(), maintainer.clone());

    let (mut update_demo, request_gallery, update_segment) =
        update_demo(object_store, git_segment_store, &owner)?;
    request_discussions::add_maintainer(&mut update_demo);
    let (published_demo, published_segment) =
        published_demo(object_store, git_segment_store, &owner)?;
    catalog
        .git_segment_uploads
        .extend([published_segment, update_segment]);
    for repo in [published_demo, update_demo] {
        catalog.repositories.insert(repo.record.id.clone(), repo);
    }
    seed_request_gallery(&mut catalog, &owner, request_gallery)?;
    #[cfg(any(test, feature = "local-dev"))]
    {
        for (repo, segment) in dependency_repositories::seed_dependency_repositories(
            object_store,
            git_segment_store,
            &owner,
        )? {
            catalog.git_segment_uploads.push(segment);
            catalog.repositories.insert(repo.record.id.clone(), repo);
        }
    }

    for repo in catalog.repositories.values() {
        let path = ScopePath::parse(scope_domain::landing_file::REPOSITORY_LANDING_FILE_PATH)
            .map_err(ApiError::internal)?;
        if let Some(blob) = repo.live_files.get(&path) {
            let bytes = scope_object_store::source_blob_bytes(object_store, blob)?;
            let landing =
                scope_domain::landing_file::RepositoryLandingFile::from_source_blob(blob, bytes)
                    .map_err(ApiError::internal)?;
            catalog
                .repository_landing_files
                .insert(repo.record.id.clone(), landing);
        }
    }
    Ok(catalog)
}

pub(crate) fn seed_user_account(seed_user: DevSeedUser) -> UserAccount {
    UserAccount {
        id: DEV_SEED_USER_ID.to_string(),
        handle: seed_user.handle,
        email: seed_user.email,
        email_verified: true,
    }
}

#[cfg(any(test, feature = "local-dev"))]
pub(crate) fn actor_account(seed_user: DevSeedUser, handle: &str) -> Option<UserAccount> {
    let owner = seed_user_account(seed_user);
    if owner.handle == handle {
        return Some(owner);
    }
    request_discussions::collaborators()
        .into_iter()
        .find(|actor| actor.handle == handle)
}

fn published_demo(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
) -> Result<(Repository, GitSegmentUpload), ApiError> {
    let mut repo = repo(owner, "public-demo", Visibility::Public)?;
    let readme = blob(object_store, PUBLIC_DEMO_README_HTML)?;
    let app = blob(object_store, PUBLIC_DEMO_APP)?;
    let private_plan = blob(object_store, PUBLIC_DEMO_PLAN)?;
    let checks_workflow = blob(object_store, PUBLIC_DEMO_CHECKS_WORKFLOW)?;
    let lint_workflow = blob(object_store, PUBLIC_DEMO_LINT_WORKFLOW)?;
    let private_path = ScopePath::parse("/internal/plan.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(private_path.clone()))
        .map_err(ApiError::internal)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-public-1",
        "Seed public demo",
        vec![
            add_change("/README.html", readme, Visibility::Public)?,
            add_change("/src/app.ts", app, Visibility::Public)?,
            add_change(
                "/.scope/runs/checks.yml",
                checks_workflow,
                Visibility::Public,
            )?,
            add_change("/.scope/runs/lint.yml", lint_workflow, Visibility::Public)?,
            add_change(private_path.as_str(), private_plan, Visibility::Private)?,
        ],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let (head, pack_span, segment_upload) = git_pack_state(
        git_segment_store,
        &repo.record.id,
        "public-demo-live",
        &[SeedGitCommit {
            files: &[
                ("README.html", PUBLIC_DEMO_README_HTML),
                ("src/app.ts", PUBLIC_DEMO_APP),
                (".scope/runs/checks.yml", PUBLIC_DEMO_CHECKS_WORKFLOW),
                (".scope/runs/lint.yml", PUBLIC_DEMO_LINT_WORKFLOW),
                ("internal/plan.md", PUBLIC_DEMO_PLAN),
            ],
            message: "Seed public demo",
        }],
    )?;
    repo.git_head = Some(head);
    repo.git_pack_spans.push(pack_span);
    Ok((repo, segment_upload))
}

fn update_demo(
    object_store: &dyn ObjectStore,
    git_segment_store: &scope_git_storage::GitSegmentStore,
    owner: &UserAccount,
) -> Result<(Repository, SeedRequestGallery, GitSegmentUpload), ApiError> {
    let mut repo = repo(owner, "update-demo", Visibility::Public)?;
    let initial_readme = blob(object_store, UPDATE_DEMO_INITIAL_README)?;
    let rules = blob(object_store, UPDATE_DEMO_RULES)?;
    let internal_notes = blob(object_store, UPDATE_DEMO_INTERNAL_NOTES)?;
    let internal_path = ScopePath::parse("/internal/notes.md").map_err(ApiError::internal)?;
    repo.policy
        .add_rule(VisibilityRule::private(internal_path.clone()))
        .map_err(ApiError::internal)?;
    let release_guide = blob(object_store, UPDATE_DEMO_RELEASE_GUIDE)?;
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-1",
        "Seed update demo",
        vec![
            add_change("/README.md", initial_readme.clone(), Visibility::Public)?,
            add_change("/.scope/RULES.md", rules, Visibility::Public)?,
            add_change(internal_path.as_str(), internal_notes, Visibility::Private)?,
        ],
    ));
    repo.graph.commits.push(commit(
        &repo,
        "dev-update-2",
        "Document release flow",
        vec![add_change(
            "/docs/release.md",
            release_guide,
            Visibility::Public,
        )?],
    ));
    populate_seed_live_files(&mut repo);
    repo.record.lifecycle_state = RepoLifecycleState::Ready;
    let initial = SeedGitCommit {
        files: &[
            ("README.md", UPDATE_DEMO_INITIAL_README),
            (".scope/RULES.md", UPDATE_DEMO_RULES),
            ("internal/notes.md", UPDATE_DEMO_INTERNAL_NOTES),
        ],
        message: "Seed update demo",
    };
    let accepted = SeedGitCommit {
        files: &[("docs/release.md", UPDATE_DEMO_RELEASE_GUIDE)],
        message: "Document release flow",
    };
    let (head, pack_span, gallery, segment_upload) = update_demo_git_snapshot(
        object_store,
        git_segment_store,
        &repo.record.id,
        initial,
        accepted,
    )?;
    repo.git_head = Some(head);
    repo.git_pack_spans.push(pack_span);
    Ok((repo, gallery, segment_upload))
}

fn populate_seed_live_files(repo: &mut Repository) {
    repo.live_files.clear();
    for change in repo.graph.commits.iter().flat_map(|commit| &commit.changes) {
        match &change.new_content {
            Some(content) => {
                repo.live_files.insert(change.path.clone(), content.clone());
            }
            None => {
                repo.live_files.remove(&change.path);
            }
        }
    }
}

type SeedRequestGallery = Vec<SeedRequest>;

struct SeedRequest {
    id: &'static str,
    name: &'static str,
    title: &'static str,
    base_oid: String,
    head_oid: String,
    snapshot: SourceBlob,
    description_markdown: Option<&'static str>,
    revisions: Vec<SeedRequestRevision>,
    outcome: SeedRequestOutcome,
    audience: RequestAudience,
    now_unix: u64,
}

enum SeedRequestOutcome {
    Draft,
    Open,
    Merged,
    Closed,
}

fn seed_request_gallery(
    catalog: &mut scope_postgres::db::CatalogFixture,
    owner: &UserAccount,
    gallery: SeedRequestGallery,
) -> Result<(), ApiError> {
    let repo_id = catalog
        .repository(&owner.handle, "update-demo")
        .ok_or_else(|| ApiError::internal_message("seeded update demo is missing"))?
        .record
        .id
        .clone();
    for request in gallery {
        seed_owner_request(catalog, owner, &repo_id, request)?;
    }
    Ok(())
}

fn seed_owner_request(
    catalog: &mut scope_postgres::db::CatalogFixture,
    owner: &UserAccount,
    repo_id: &str,
    request: SeedRequest,
) -> Result<(), ApiError> {
    let SeedRequest {
        id,
        name,
        title,
        base_oid,
        head_oid,
        snapshot,
        description_markdown,
        revisions,
        outcome,
        audience,
        now_unix,
    } = request;
    let started = start_request(
        StartRequestFacts {
            request_id_exists: catalog.requests.contains_key(id),
            request_name_exists: catalog
                .requests
                .values()
                .any(|request| request.repo_id == repo_id && request.name == name),
            public_working_request_count: 0,
        },
        StartRequestInput {
            id: id.to_string(),
            repo_id: repo_id.to_string(),
            author_user_id: owner.id.clone(),
            name: name.to_string(),
            title: Some(title.to_string()),
            author_role: RequestActorRole::Owner,
            audience,
            base_main_oid: base_oid.clone(),
            event_id: format!("event_{id}_started"),
            now_unix,
        },
    )?;
    catalog
        .request_events
        .insert(started.event.id.clone(), started.event);
    let uploaded = record_working_request_upload(
        started.request,
        RecordWorkingRequestUploadInput {
            request_id: id.to_string(),
            actor_user_id: owner.id.clone(),
            actor_can_edit: true,
            expected_old_head_oid: None,
            new_head_oid: head_oid.clone(),
            git_snapshot: snapshot,
            now_unix: now_unix + 1,
        },
    )?;
    let mut request = uploaded.request;
    if let Some(description_markdown) = description_markdown {
        let event_id = format!("event_{id}_identity_edited");
        let mutation = edit_request_identity(
            request,
            catalog.request_events.contains_key(&event_id),
            EditRequestIdentityInput {
                request_id: id.to_string(),
                actor_user_id: owner.id.clone(),
                actor_can_edit_identity: true,
                event_id,
                title: None,
                description_markdown: Some(description_markdown.to_string()),
                expected_description_markdown: None,
                now_unix: now_unix + 2,
            },
        )?;
        request = mutation.request;
        catalog
            .request_events
            .insert(mutation.event.id.clone(), mutation.event);
    }
    let mut current_head_oid = head_oid.clone();
    let mut lifecycle_at_unix = now_unix + 2;
    for (index, revision) in revisions.into_iter().enumerate() {
        let event_id = format!("event_{id}_revision_{}", index + 1);
        let mutation = record_request_revision(
            request,
            catalog.request_events.contains_key(&event_id),
            RecordRequestRevisionInput {
                request_id: id.to_string(),
                actor_user_id: owner.id.clone(),
                actor_can_edit: true,
                expected_old_head_oid: Some(current_head_oid),
                new_head_oid: revision.head_oid.clone(),
                git_snapshot: revision.snapshot,
                event_id,
                body: Some(revision.note.to_string()),
                now_unix: now_unix + 3 + index as u64,
            },
        )?;
        request = mutation.request;
        catalog
            .request_events
            .insert(mutation.event.id.clone(), mutation.event);
        current_head_oid = revision.head_oid;
        lifecycle_at_unix = now_unix + 4 + index as u64;
        catalog
            .request_revisions
            .insert(mutation.revision.id.clone(), mutation.revision);
    }

    if !matches!(outcome, SeedRequestOutcome::Draft) {
        request.submitted_at_unix = Some(lifecycle_at_unix);
    }
    match outcome {
        SeedRequestOutcome::Draft | SeedRequestOutcome::Open => {}
        SeedRequestOutcome::Merged => {
            request.merged_at_unix = Some(lifecycle_at_unix + 1);
            request.merged_by_user_id = Some(owner.id.clone());
            request.merged_head_oid = Some(current_head_oid.clone());
            request.merged_main_oid = Some(current_head_oid);
        }
        SeedRequestOutcome::Closed => {
            request.closed_at_unix = Some(lifecycle_at_unix + 1);
            request.closed_by_user_id = Some(owner.id.clone());
        }
    }
    request.updated_at_unix = match outcome {
        SeedRequestOutcome::Draft | SeedRequestOutcome::Open => lifecycle_at_unix,
        SeedRequestOutcome::Merged | SeedRequestOutcome::Closed => lifecycle_at_unix + 1,
    };
    request.validate_facts()?;
    catalog.requests.insert(request.id.clone(), request);
    Ok(())
}

fn repo(owner: &UserAccount, name: &str, visibility: Visibility) -> Result<Repository, ApiError> {
    Repository::new(
        owner,
        name,
        visibility,
        format!(
            "repoi_seed_{}",
            scope_domain::repository::repo_id(&owner.handle, name)
        ),
    )
    .map_err(|error| ApiError::internal_message(error.to_string()))
}

fn commit(repo: &Repository, id: &str, message: &str, changes: Vec<FileChange>) -> LogicalCommit {
    LogicalCommit {
        occurred_at_unix: None,
        id: id.to_string(),
        origin: LogicalCommitOrigin::CanonicalPush {
            source_head_oid: id.to_string(),
        },
        author_id: repo.record.owner_user_id.clone(),
        message: message.to_string(),
        changes,
    }
}

fn add_change(
    path: &str,
    new_content: SourceBlob,
    visibility: Visibility,
) -> Result<FileChange, ApiError> {
    Ok(FileChange {
        path: ScopePath::parse(path).map_err(ApiError::internal)?,
        old_content: None,
        new_content: Some(new_content),
        visibility,
    })
}

fn blob(object_store: &dyn ObjectStore, content: &str) -> Result<SourceBlob, ApiError> {
    Ok(put_source_blob(object_store, content.as_bytes())?)
}
