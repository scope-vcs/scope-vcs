use crate::{
    config::EMPTY_GIT_OID,
    error::ApiError,
    git::{
        command::{git_ref_listing, run_git, run_git_output},
        import::{git_snapshot_from_ref, validate_pushed_commit_range},
        repository_git::RepositoryGit,
        request_ref_public_safety::ensure_public_request_ref_is_public_safe,
        staging::write_receive_pack_hook,
        storage::{
            receive_pack_staging_repo_path, remove_dir_if_exists, request_ref_store_repo_path,
        },
    },
    state::AppState,
};
use scope_domain::{
    content::SourceBlob,
    repository::RepositoryIncarnation,
    requests::{
        Request, RequestAudience, RequestRevisionGitFacts, canonical_request_ref,
        request_base_after_revision,
    },
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path as FsPath, PathBuf},
};

mod ancestry;
mod cleanup;
pub(crate) use cleanup::cleanup_deleted_request_ref;
mod locks;
mod revision;
mod snapshot;
#[cfg(test)]
use crate::persistence::unix_now;
use ancestry::{
    ensure_request_ref_descends_from_base, request_ref_oid_is_commit, request_revision_git_facts,
    thin_snapshot_base,
};
use locks::acquire_request_ref_store_lock;
pub(crate) use locks::acquire_request_ref_update_lock_async;
pub(crate) use revision::with_request_revision_store_repo;
#[cfg(test)]
use snapshot::bundle_prerequisites;
use snapshot::{download_snapshot, fetch_bundle_into, fetch_snapshot_into};

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

pub(crate) fn create_request_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    seed_repo: &FsPath,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-hardlinks",
            seed_repo.to_string_lossy().as_ref(),
            repo_root.to_string_lossy().as_ref(),
        ],
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
    public_base_repo: Option<&FsPath>,
) -> Result<(), ApiError> {
    for request in requests {
        let request_ref = canonical_request_ref(&request.name);
        if let Some(snapshot) = request.git_snapshot.as_ref() {
            fetch_snapshot_into(
                state,
                target_repo,
                &request_ref,
                snapshot,
                None,
                "attaching request ref to Git read view",
            )?;
        } else {
            if !request_ref_oid_is_commit(target_repo, &request.head_oid)?
                && let Some(public_base_repo) = public_base_repo
            {
                let temporary_ref = "refs/scope/internal/public-request-base";
                let refspec = format!("+refs/heads/{DEFAULT_GIT_BRANCH}:{temporary_ref}");
                run_git(
                    Some(target_repo),
                    &[
                        "fetch",
                        public_base_repo.to_string_lossy().as_ref(),
                        &refspec,
                    ],
                    "attaching public request base to Git read view",
                )?;
                run_git(
                    Some(target_repo),
                    &["update-ref", "-d", temporary_ref],
                    "removing temporary public request base ref",
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
    pub(crate) previous_head: Option<String>,
    pub(crate) git_snapshot: SourceBlob,
    pub(crate) git_facts: RequestRevisionGitFacts,
    pub(crate) fence: scope_postgres::db::ContentRefFence,
}

pub(crate) async fn persist_request_ref_to_store(
    state: &AppState,
    git: &RepositoryGit,
    staging_repo: &FsPath,
    request: &Request,
    update: &RequestRefUpdate,
) -> Result<PersistedRequestRef, ApiError> {
    let path = staging_repo.to_path_buf();
    let base_oid = request.base_main_oid.clone();
    let head_oid = update.new_head_oid.clone();
    crate::git::blocking::run(move || {
        ensure_request_ref_oid_is_commit(&path, &head_oid)?;
        ensure_request_ref_descends_from_base(&path, &base_oid, &head_oid)
    })
    .await?;
    let accepted_main_oid = git.git_head.as_ref().map(|head| head.head_oid.clone());
    let main_oid = match request.audience {
        RequestAudience::Public => Some(
            ensure_public_request_ref_is_public_safe(
                git,
                state,
                staging_repo,
                &update.new_head_oid,
            )
            .await?,
        ),
        RequestAudience::Private => accepted_main_oid.clone(),
    };
    let incarnation = git.incarnation.clone();
    let prepared = {
        let state = state.clone();
        let incarnation = incarnation.clone();
        let path = staging_repo.to_path_buf();
        let request = request.clone();
        let update = update.clone();
        crate::git::blocking::run(move || {
            prepare_request_ref_snapshot(
                &state,
                &incarnation,
                &path,
                &request,
                &update,
                RequestMainTips {
                    accepted: accepted_main_oid.as_deref(),
                    audience: main_oid.as_deref(),
                },
            )
        })
        .await?
    };
    let PreparedRequestRef {
        store_lock,
        previous_head,
        git_snapshot,
        git_facts,
        snapshot_bytes,
    } = prepared;
    let fence = match state
        .metadata
        .acquire_content_ref_fence(std::slice::from_ref(&git_snapshot.content_ref))
        .await
    {
        Ok(fence) => fence,
        Err(error) => {
            let state = state.clone();
            let request_ref = update.request_ref.clone();
            crate::git::blocking::run(move || {
                let _store_lock = store_lock;
                rollback_request_ref(&state, &incarnation, &request_ref, previous_head);
                Ok(())
            })
            .await?;
            return Err(error.into());
        }
    };
    let result = {
        let state = state.clone();
        let request_ref = update.request_ref.clone();
        let previous_head = previous_head.clone();
        let object_key = scope_storage::object_key(&git_snapshot);
        crate::git::blocking::run(move || {
            let _store_lock = store_lock;
            if let Err(error) =
                crate::git::blocking::block_on(state.object_store.put(&object_key, snapshot_bytes))
            {
                rollback_request_ref(&state, &incarnation, &request_ref, previous_head);
                return Err(error.into());
            }
            Ok(())
        })
        .await
    };
    if let Err(error) = result {
        fence.release().await;
        return Err(error);
    }
    Ok(PersistedRequestRef {
        previous_head,
        git_snapshot,
        git_facts,
        fence,
    })
}

struct PreparedRequestRef {
    store_lock: locks::GitLockFile,
    previous_head: Option<String>,
    git_snapshot: SourceBlob,
    git_facts: RequestRevisionGitFacts,
    snapshot_bytes: Vec<u8>,
}

struct RequestMainTips<'a> {
    accepted: Option<&'a str>,
    audience: Option<&'a str>,
}

fn prepare_request_ref_snapshot(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    staging_repo: &FsPath,
    request: &Request,
    update: &RequestRefUpdate,
    main_tips: RequestMainTips<'_>,
) -> Result<PreparedRequestRef, ApiError> {
    let store_lock = acquire_request_ref_store_lock(state, incarnation)?;
    let store_repo = ensure_request_ref_store_repo_locked(state, incarnation)?;
    ensure_request_ref_available_in_store_locked(state, &store_repo, staging_repo, request)?;
    let previous_head = request_ref_head(&store_repo, &update.request_ref)?;
    let expected_stored_head = previous_head.as_deref().or_else(|| {
        request
            .git_snapshot
            .is_none()
            .then_some(request.head_oid.as_str())
    });
    let logical_old_head = update
        .old_head_oid
        .as_deref()
        .unwrap_or(request.head_oid.as_str());
    validate_pushed_commit_range(staging_repo, Some(logical_old_head), &update.new_head_oid)?;
    ensure_request_ref_store_head_matches_push(expected_stored_head, Some(logical_old_head))?;
    let git_facts = request_revision_git_facts(
        staging_repo,
        &request.base_main_oid,
        logical_old_head,
        &update.new_head_oid,
        main_tips.audience,
    )?;
    let snapshot_base = thin_snapshot_base(
        request.audience,
        request_base_after_revision(request, &git_facts),
        main_tips.accepted,
        staging_repo,
    )?;
    let refspec = format!("+{}:{}", update.request_ref, update.request_ref);
    run_git(
        Some(&store_repo),
        &["fetch", staging_repo.to_string_lossy().as_ref(), &refspec],
        "persisting request ref",
    )?;
    let (git_snapshot, snapshot_bytes) =
        match git_snapshot_from_ref(&store_repo, &update.request_ref, snapshot_base) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                rollback_request_ref(state, incarnation, &update.request_ref, previous_head);
                return Err(error);
            }
        };
    Ok(PreparedRequestRef {
        store_lock,
        previous_head,
        git_snapshot,
        git_facts,
        snapshot_bytes,
    })
}

fn ensure_request_ref_oid_is_commit(repo: &FsPath, oid: &str) -> Result<(), ApiError> {
    if request_ref_oid_is_commit(repo, oid)? {
        return Ok(());
    }
    Err(ApiError::bad_request(REQUEST_REF_COMMIT_ERROR))
}

fn ensure_request_ref_available_in_store_locked(
    state: &AppState,
    store_repo: &FsPath,
    staging_repo: &FsPath,
    request: &Request,
) -> Result<(), ApiError> {
    let request_ref = canonical_request_ref(&request.name);
    if request_ref_head(store_repo, &request_ref)?.as_deref() == Some(request.head_oid.as_str()) {
        return Ok(());
    }
    if let Some(snapshot) = request.git_snapshot.as_ref() {
        fetch_snapshot_into(
            state,
            store_repo,
            &request_ref,
            snapshot,
            Some(staging_repo),
            "restoring request ref snapshot",
        )?;
        if request_ref_head(store_repo, &request_ref)?.as_deref() == Some(request.head_oid.as_str())
        {
            return Ok(());
        }
        return Err(ApiError::infrastructure_unavailable(
            "stored request branch snapshot does not match request metadata",
        ));
    }
    if request_ref_exists(store_repo, &request_ref)? {
        run_git(
            Some(store_repo),
            &["update-ref", "-d", &request_ref],
            "deleting stale request ref cache",
        )?;
    }
    Ok(())
}

fn ensure_request_ref_store_head_matches_push(
    stored_head: Option<&str>,
    advertised_old_head: Option<&str>,
) -> Result<(), ApiError> {
    if stored_head == advertised_old_head {
        return Ok(());
    }
    Err(ApiError::conflict(
        "request branch changed since push started; fetch and retry",
    ))
}

fn ensure_request_ref_store_repo_locked(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let store_repo = request_ref_store_repo_path(state, incarnation);
    if store_repo.join("objects").is_dir() {
        return Ok(store_repo);
    }
    if store_repo.exists() {
        remove_dir_if_exists(&store_repo)?;
    }
    if let Some(parent) = store_repo.parent() {
        crate::persistence::ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &["init", "--bare", store_repo.to_string_lossy().as_ref()],
        "initializing request ref store",
    )?;
    run_git(
        Some(&store_repo),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting request ref store head",
    )?;
    Ok(store_repo)
}

fn request_ref_exists(store_repo: &FsPath, request_ref: &str) -> Result<bool, ApiError> {
    Ok(request_ref_head(store_repo, request_ref)?.is_some())
}

fn request_ref_head(store_repo: &FsPath, request_ref: &str) -> Result<Option<String>, ApiError> {
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

pub(crate) fn rollback_request_ref(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    request_ref: &str,
    previous_head: Option<String>,
) {
    let store_repo = request_ref_store_repo_path(state, incarnation);
    let result = match previous_head {
        Some(head) => run_git(
            Some(&store_repo),
            &["update-ref", request_ref, &head],
            "rolling back request ref",
        ),
        None => {
            if store_repo.exists() {
                run_git(
                    Some(&store_repo),
                    &["update-ref", "-d", request_ref],
                    "deleting rolled-back request ref",
                )
            } else {
                Ok(())
            }
        }
    };
    if let Err(error) = result {
        tracing::warn!(
            repository_id = incarnation.repository_id(),
            repository_incarnation_id = incarnation.incarnation_id(),
            request_ref,
            error = error.operator_diagnostic(),
            "failed to roll back request ref after metadata rejection"
        );
    }
}

#[cfg(test)]
mod tests;
