use crate::{
    config::EMPTY_GIT_OID,
    error::ApiError,
    git::{
        command::run_git,
        projection_repo::projection_bare_repo_for_state,
        request_refs::{REQUEST_REF_COMMIT_ERROR, REQUEST_REF_FAST_FORWARD_ERROR},
        storage::receive_pack_staging_repo_path,
    },
    persistence::ensure_private_dir,
    repo_access::find_repo,
    state::AppState,
};
use scope_domain::{
    policy::Principal,
    projection::{ProjectionViewKey, project_graph},
    repository::{RepoLifecycleState, RepositoryIncarnation},
};
use scope_git::DEFAULT_GIT_BRANCH;
use std::{
    fs,
    path::{Path as FsPath, PathBuf},
};

pub(crate) async fn ensure_first_push_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let state = state.clone();
    let incarnation = incarnation.clone();
    crate::git::blocking::run(move || initialize_first_push_staging_repo(&state, &incarnation))
        .await
}

fn initialize_first_push_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
) -> Result<PathBuf, ApiError> {
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    run_git(
        None,
        &["init", "--bare", repo_root.to_string_lossy().as_ref()],
        "initializing receive-pack staging repo",
    )?;
    run_git(
        Some(&repo_root),
        &["config", "http.receivepack", "true"],
        "enabling receive-pack",
    )?;
    run_git(
        Some(&repo_root),
        &[
            "symbolic-ref",
            "HEAD",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "setting receive-pack default branch",
    )?;
    install_first_push_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
}

pub(crate) async fn ensure_ready_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    owner: &str,
    repo_name: &str,
    author_id: &str,
) -> Result<PathBuf, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .git_push_context(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    if repo.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::conflict("repo must be ready before push"));
    }
    if repo.incarnation != *incarnation {
        return Err(ApiError::conflict(
            "repository was recreated during push preparation",
        ));
    }
    let seed_repo = if let Some(head) = repo.git_head.as_ref() {
        state
            .repository_engine
            .materialize_repository(state, incarnation, head, &repo.git_pack_spans)
            .await?
    } else {
        let repo = find_repo(state, owner, repo_name).await?;
        let principal = Principal {
            id: author_id.to_string(),
            kind: scope_domain::policy::PrincipalKind::User,
        };
        let view_key = ProjectionViewKey::from_access(repo.access_for_principal(&principal));
        let projection = project_graph(&repo.graph, &repo.visibility_change_sets, view_key);
        projection_bare_repo_for_state(
            state,
            incarnation,
            &projection,
            repo.git_head.as_ref(),
            &repo.git_pack_spans,
        )
        .await?
    };
    let state = state.clone();
    let incarnation = incarnation.clone();
    crate::git::blocking::run(move || {
        let repo_root = receive_pack_staging_repo_path(&state, &incarnation)?;
        if let Some(parent) = repo_root.parent() {
            ensure_private_dir(parent)?;
        }
        // Local cloning hardlinks/copies objects, so the staging repository does
        // not retain alternates into a cache that can be evicted after this lease.
        run_git(
            None,
            &[
                "clone",
                "--bare",
                "--local",
                seed_repo.to_string_lossy().as_ref(),
                repo_root.to_string_lossy().as_ref(),
            ],
            "cloning receive-pack staging repo",
        )?;
        run_git(
            Some(&repo_root),
            &["config", "http.receivepack", "true"],
            "enabling receive-pack",
        )?;
        install_ready_pre_receive_hook(&repo_root)?;
        Ok(repo_root)
    })
    .await
}

pub(crate) fn install_first_push_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        "#!/bin/sh\ncount=0\nwhile read old new ref; do\n  count=$((count + 1))\n  if [ \"$ref\" != \"refs/heads/{DEFAULT_GIT_BRANCH}\" ]; then\n    echo \"Scope accepts pushes only to refs/heads/{DEFAULT_GIT_BRANCH}\" >&2\n    exit 1\n  fi\n  if [ \"$new\" = \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope does not accept branch deletes in v0\" >&2\n    exit 1\n  fi\n  if [ \"$old\" != \"{EMPTY_GIT_OID}\" ]; then\n    echo \"Scope accepts only the initial branch push in v0\" >&2\n    exit 1\n  fi\ndone\nif [ \"$count\" -ne 1 ]; then\n  echo \"Scope accepts exactly one pushed branch in v0\" >&2\n  exit 1\nfi\n"
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn install_ready_pre_receive_hook(repo_root: &FsPath) -> Result<(), ApiError> {
    let hook = repo_root.join("hooks").join("pre-receive");
    let script = format!(
        r#"#!/bin/sh
count=0
while read old new ref; do
  count=$((count + 1))
  if [ "$new" = "{EMPTY_GIT_OID}" ]; then
    echo "Scope does not accept branch deletes" >&2
    exit 1
  fi
  if [ "$ref" = "refs/heads/{DEFAULT_GIT_BRANCH}" ]; then
    if [ "$old" = "{EMPTY_GIT_OID}" ]; then
      echo "Scope accepts only updates to refs/heads/{DEFAULT_GIT_BRANCH}" >&2
      exit 1
    fi
    if ! git merge-base --is-ancestor "$old" "$new"; then
      echo "Scope rejects non-fast-forward pushes" >&2
      exit 1
    fi
    continue
  fi
  case "$ref" in
    refs/heads/*)
      if ! git cat-file -e "$new^{{commit}}"; then
        echo "{REQUEST_REF_COMMIT_ERROR}" >&2
        exit 1
      fi
      if [ "$old" != "{EMPTY_GIT_OID}" ] && ! git merge-base --is-ancestor "$old" "$new"; then
        echo "{REQUEST_REF_FAST_FORWARD_ERROR}" >&2
        exit 1
      fi
      ;;
    *)
      echo "Scope accepts pushes only to main or a named request branch" >&2
      exit 1
      ;;
  esac
done
if [ "$count" -ne 1 ]; then
  echo "Scope accepts exactly one pushed ref" >&2
  exit 1
fi
"#
    );
    write_receive_pack_hook(&hook, &script)
}

pub(crate) fn write_receive_pack_hook(hook: &FsPath, script: &str) -> Result<(), ApiError> {
    fs::write(hook, script).map_err(ApiError::internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(hook)
            .map_err(ApiError::internal)?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(hook, permissions).map_err(ApiError::internal)?;
    }
    Ok(())
}
