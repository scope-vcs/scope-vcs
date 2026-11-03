use crate::{
    config::EMPTY_GIT_OID,
    error::ApiError,
    git::{
        command::{git_ref_listing, run_git, run_git_output},
        import::{git_snapshot_from_ref, validate_pushed_commit_range},
        pool_pack::{PoolPackRevision, StagedPoolPack, stage_pool_pack},
        repository_git::RepositoryGit,
        request_ref_view_safety::{RequestView, ensure_request_ref_is_view_safe},
        staging::write_receive_pack_hook,
        storage::receive_pack_staging_repo_path,
    },
    state::AppState,
};
use scope_domain::{
    content::SourceBlob,
    repo_config::RepoConfig,
    repository::{
        RepositoryIncarnation,
        git::{GitPoolPackOwner, RequestPackScope},
    },
    requests::{
        Request, RequestRevisionGitFacts, canonical_request_ref,
        request_main_ancestor_after_revision,
    },
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
};

mod ancestry;
mod pack_consolidation;
pub(super) mod pool;
mod revision;
mod seeding;
mod snapshot;
mod view_bases;
pub(super) mod view_pool;
pub(crate) use ancestry::request_revision_git_facts;
use ancestry::{
    ensure_request_ref_descends_from_base, request_ref_oid_is_commit, thin_snapshot_base,
};
#[cfg(test)]
pub(crate) use pool::request_object_repos;
pub(crate) use pool::{
    PoolInstall, PoolRepos, RequestObjectRepos, install_missing_request_packs,
    retain_staged_pool_pack,
};
pub(crate) use revision::{sweep_revision_jobs, with_request_revision_store_repo};
pub(crate) use seeding::{
    DeferredRequestObjects, attach_request_refs_from_pool, attach_view_request_refs_from_pool,
};
#[cfg(test)]
use snapshot::fetch_bundle_into_ref;
use snapshot::{BundleRef, fetch_snapshot_into_ref};
pub(crate) use snapshot::{bundle_prerequisites, download_snapshot};
pub(crate) use view_bases::{RequestViewBases, request_view_bases};
#[cfg(test)]
pub(crate) use view_pool::view_object_repos;

pub(crate) const REQUEST_REF_DELETE_ERROR: &str = "Scope does not accept request branch deletes";
pub(crate) const REQUEST_REF_SINGLE_UPDATE_ERROR: &str =
    "Scope accepts exactly one request ref update";
pub(crate) const REQUEST_REF_COMMIT_ERROR: &str = "Scope request refs must point at commits";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RequestRefUpdate {
    pub(crate) request_ref: String,
    pub(crate) request_name: String,
    pub(crate) old_head_oid: Option<String>,
    pub(crate) new_head_oid: String,
}

fn request_name_from_ref(refname: &str) -> Option<&str> {
    let name = refname.strip_prefix("refs/heads/")?;
    (!name.is_empty() && name != DEFAULT_GIT_BRANCH && !name.contains('/')).then_some(name)
}

fn is_request_ref_candidate(refname: &str) -> bool {
    request_name_from_ref(refname).is_some()
}

pub(crate) fn receive_pack_refs(staging_repo: &FsPath) -> Result<Vec<(String, String)>, ApiError> {
    git_ref_listing(
        staging_repo,
        &["refs/heads", "refs/tags"],
        "reading receive-pack refs",
    )
}

pub(crate) fn request_ref_update_from_refs(
    refs_before: &[(String, String)],
    refs_after: &[(String, String)],
) -> Result<Option<RequestRefUpdate>, ApiError> {
    let before = refs_by_name(refs_before);
    let after = refs_by_name(refs_after);
    let mut changed = Vec::new();

    for refname in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
        if !is_request_ref_candidate(refname) {
            continue;
        }
        let old = before.get(refname);
        let new = after.get(refname);
        if old == new {
            continue;
        }
        let Some(new_head_oid) = new else {
            return Err(ApiError::bad_request(REQUEST_REF_DELETE_ERROR));
        };
        let request_name =
            request_name_from_ref(refname).expect("request ref was classified above");
        scope_domain::requests::validate_request_name(request_name).map_err(|error| {
            ApiError::bad_request(format!(
                "invalid request branch '{request_name}': {}",
                error.message
            ))
        })?;
        changed.push(RequestRefUpdate {
            request_ref: refname.clone(),
            request_name: request_name.to_string(),
            old_head_oid: old.cloned(),
            new_head_oid: new_head_oid.clone(),
        });
    }

    match changed.len() {
        0 => Ok(None),
        1 => Ok(changed.pop()),
        _ => Err(ApiError::bad_request(REQUEST_REF_SINGLE_UPDATE_ERROR)),
    }
}

pub(crate) fn non_request_refs_changed(
    refs_before: &[(String, String)],
    refs_after: &[(String, String)],
) -> bool {
    let before = refs_by_name(refs_before);
    let after = refs_by_name(refs_after);
    before
        .keys()
        .chain(after.keys())
        .filter(|refname| !is_request_ref_candidate(refname))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .any(|refname| before.get(refname) != after.get(refname))
}

pub(crate) fn clone_without_shared_objects(
    seed_repo: &FsPath,
    repo_root: &FsPath,
    context: &str,
) -> Result<(), ApiError> {
    let seed = seed_repo.to_string_lossy();
    let target = repo_root.to_string_lossy();
    let mut args = vec!["clone", "--bare"];
    if seed_repo.join("objects/info/alternates").is_file() {
        args.push("--no-local");
    }
    args.extend([seed.as_ref(), target.as_ref()]);
    run_git(None, &args, context)
}

pub(crate) fn create_request_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    seed_repo: &FsPath,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    clone_without_shared_objects(
        seed_repo,
        &repo_root,
        "cloning request receive-pack staging repo",
    )?;
    if let Err(error) = run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling request receive-pack",
    ) {
        let _ = fs::remove_dir_all(&repo_root);
        return Err(error);
    }
    Ok(repo_root)
}

pub(crate) fn attach_visible_request_refs(
    state: &AppState,
    requests: &[Request],
    target_repo: &FsPath,
    view_bases: &RequestViewBases,
) -> Result<(), ApiError> {
    for request in requests
        .iter()
        .filter(|request| request.git_snapshot.is_some())
        .chain(
            requests
                .iter()
                .filter(|request| request.git_snapshot.is_none()),
        )
    {
        let request_ref = canonical_request_ref(&request.name);
        if let Some(snapshot) = request.git_snapshot.as_ref() {
            fetch_snapshot_into_ref(
                state,
                target_repo,
                BundleRef {
                    source: &request_ref,
                    destination: &request_ref,
                },
                snapshot,
                None,
                "attaching request ref to Git read view",
            )?;
        } else {
            if !request_ref_oid_is_commit(target_repo, &request.head_oid)?
                && let Some(view_base) = view_bases.get(&request.view)
            {
                let temporary_ref = "refs/scope/internal/request-view-base";
                let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{temporary_ref}");
                run_git(
                    Some(target_repo),
                    &["fetch", view_base.to_string_lossy().as_ref(), &refspec],
                    "attaching request view base to Git read view",
                )?;
                run_git(
                    Some(target_repo),
                    &["update-ref", "-d", temporary_ref],
                    "removing temporary request view base ref",
                )?;
            }
            if !request_ref_oid_is_commit(target_repo, &request.head_oid)? {
                tracing::warn!(
                    request_id = request.id,
                    request_name = request.name,
                    head_oid = request.head_oid,
                    "omitting snapshotless request whose base commit is unavailable in Git read view"
                );
                continue;
            }
            run_git(
                Some(target_repo),
                &["update-ref", &request_ref, &request.head_oid],
                "attaching unmodified request ref to Git read view",
            )?;
        }
        let attached_head = request_ref_head(target_repo, &request_ref)?;
        if attached_head.as_deref() != Some(request.head_oid.as_str()) {
            return Err(ApiError::infrastructure_unavailable(
                "request snapshot does not match request metadata",
            ));
        }
    }
    Ok(())
}

fn refs_by_name(refs: &[(String, String)]) -> BTreeMap<String, String> {
    refs.iter()
        .map(|(refname, oid)| (refname.clone(), oid.clone()))
        .collect()
}

pub(crate) fn install_request_receive_pack_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        r#"#!/bin/sh
count=0
while read old new ref; do
  count=$((count + 1))
  case "$ref" in
    refs/heads/{DEFAULT_GIT_BRANCH})
      echo "Scope contributors cannot update main" >&2
      exit 1
      ;;
    refs/heads/*) ;;
    *)
      echo "Scope request pushes only accept named request branches" >&2
      exit 1
      ;;
  esac
  if [ "$new" = "{EMPTY_GIT_OID}" ]; then
    echo "{REQUEST_REF_DELETE_ERROR}" >&2
    exit 1
  fi
  if [ "$(git cat-file -t "$new" 2>/dev/null)" != "commit" ]; then
    echo "{REQUEST_REF_COMMIT_ERROR}" >&2
    exit 1
  fi
done
if [ "$count" -ne 1 ]; then
  echo "{REQUEST_REF_SINGLE_UPDATE_ERROR}" >&2
  exit 1
fi
"#
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) struct PersistedRequestRef {
    pub(crate) git_snapshot: SourceBlob,
    pub(crate) git_facts: RequestRevisionGitFacts,
    pub(crate) fence: scope_postgres::db::ContentRefFence,
    pub(crate) pool_pack: StagedPoolPack,
}

pub(crate) async fn persist_request_ref(
    state: &AppState,
    git: &RepositoryGit,
    repo_config: &RepoConfig,
    staging_repo: &FsPath,
    request: &Request,
    update: &RequestRefUpdate,
    scope: RequestPackScope,
) -> Result<PersistedRequestRef, ApiError> {
    let path = staging_repo.to_path_buf();
    let base_oid = request.base.review_base_oid().to_string();
    let main_ancestor_oid = request.main_ancestor_oid.clone();
    let head_oid = update.new_head_oid.clone();
    crate::git::blocking::run(move || {
        ensure_request_ref_oid_is_commit(&path, &head_oid)?;
        ensure_request_ref_descends_from_base(&path, &main_ancestor_oid, &head_oid)?;
        ensure_request_ref_descends_from_base(&path, &base_oid, &head_oid)
    })
    .await?;
    let accepted_main_oid = git.git_head.as_ref().map(|head| head.head_oid.clone());
    let views = repo_config.views();
    let main_oid = if &request.view == views.full() {
        accepted_main_oid.clone()
    } else {
        Some(
            ensure_request_ref_is_view_safe(
                RequestView::new(git, views, &request.view),
                repo_config,
                state,
                staging_repo,
                &update.new_head_oid,
            )
            .await?,
        )
    };
    let (git_snapshot, git_facts, snapshot_bytes, pool_prerequisites) = {
        let path = staging_repo.to_path_buf();
        let request = request.clone();
        let update = update.clone();
        let scope = scope.clone();
        crate::git::blocking::run(move || {
            let logical_old_head = update
                .old_head_oid
                .as_deref()
                .unwrap_or(request.head_oid.as_str());
            validate_pushed_commit_range(&path, Some(logical_old_head), &update.new_head_oid)?;
            let git_facts = request_revision_git_facts(
                &path,
                &request.main_ancestor_oid,
                logical_old_head,
                &update.new_head_oid,
                main_oid.as_deref(),
            )?;
            let snapshot_base = thin_snapshot_base(
                request.view.clone(),
                request_main_ancestor_after_revision(&request, &git_facts),
                accepted_main_oid.as_deref(),
                &path,
            )?;
            let (git_snapshot, snapshot_bytes) =
                git_snapshot_from_ref(&path, &update.request_ref, snapshot_base)?;
            let pool_main = match scope {
                RequestPackScope::Full => accepted_main_oid.as_deref(),
                RequestPackScope::View(_) => main_oid.as_deref(),
            };
            let pool_prerequisites = [Some(logical_old_head), pool_main]
                .into_iter()
                .flatten()
                .filter(|oid| *oid != EMPTY_GIT_OID)
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .filter(|oid| request_ref_oid_is_commit(&path, oid).unwrap_or(false))
                .collect();
            Ok((git_snapshot, git_facts, snapshot_bytes, pool_prerequisites))
        })
        .await?
    };
    let pool_pack = stage_pool_pack(
        state,
        git.incarnation.repository_id(),
        staging_repo,
        PoolPackRevision {
            owner: GitPoolPackOwner::Request {
                request_id: request.id.clone(),
                scope,
            },
            head_oid: update.new_head_oid.clone(),
            prerequisites: pool_prerequisites,
        },
    )
    .await?;
    let stored = async {
        let fence = state
            .metadata
            .acquire_content_ref_fence(std::slice::from_ref(&git_snapshot.content_ref))
            .await?;
        let object_key = scope_storage::object_key(&git_snapshot);
        if let Err(error) = state.object_store.put(&object_key, snapshot_bytes).await {
            fence.release().await;
            return Err(ApiError::from(error));
        }
        Ok(fence)
    }
    .await;
    let fence = match stored {
        Ok(fence) => fence,
        Err(error) => {
            pool_pack
                .discard(state, git.incarnation.repository_id())
                .await;
            return Err(error);
        }
    };
    Ok(PersistedRequestRef {
        git_snapshot,
        git_facts,
        fence,
        pool_pack,
    })
}

fn ensure_request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<(), ApiError> {
    if request_ref_oid_is_commit(repo, oid)? {
        return Ok(());
    }
    Err(ApiError::bad_request(REQUEST_REF_COMMIT_ERROR))
}

pub(crate) fn ensure_local_bare_repo(repo: PathBuf) -> Result<PathBuf, ApiError> {
    if repo.join("objects").is_dir() {
        return Ok(repo);
    }
    let parent = repo
        .parent()
        .ok_or_else(|| ApiError::internal_message("local Git repository has no parent"))?;
    crate::persistence::ensure_private_dir(parent)?;
    let initializing = tempfile::Builder::new()
        .prefix(".initializing-")
        .tempdir_in(parent)
        .map_err(ApiError::internal)?;
    let initialized = (|| {
        run_git(
            None,
            &[
                "init",
                "--bare",
                "--quiet",
                initializing.path().to_string_lossy().as_ref(),
            ],
            "initializing local Git repository",
        )?;
        run_git(
            Some(initializing.path()),
            &[
                "symbolic-ref",
                "HEAD",
                &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
            ],
            "setting local Git repository head",
        )?;
        run_git(
            Some(initializing.path()),
            &["config", "gc.auto", "0"],
            "keeping local Git repository objects",
        )?;
        fs::rename(initializing.path(), &repo).map_err(ApiError::internal)
    })();
    match initialized {
        Ok(()) => Ok(repo),
        Err(error) => {
            if repo.join("objects").is_dir() {
                Ok(repo)
            } else {
                Err(error)
            }
        }
    }
}

pub(crate) fn request_ref_head(
    store_repo: &FsPath,
    request_ref: &str,
) -> Result<Option<String>, ApiError> {
    if !store_repo.exists() {
        return Ok(None);
    }
    let output = run_git_output(
        Some(store_repo),
        &["rev-parse", "--verify", "--quiet", request_ref],
        "reading stored request ref",
    )?;
    if output.status.success() {
        let head = String::from_utf8(output.stdout).map_err(ApiError::bad_request)?;
        return Ok(Some(head.trim().to_string()));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(ApiError::infrastructure_unavailable(format!(
        "reading stored request ref: {}",
        crate::git::command::truncated_git_stderr(&output.stderr).trim(),
    )))
}

#[cfg(test)]
mod tests;
