use crate::{
    config::EMPTY_GIT_OID,
    error::ApiError,
    git::{
        cache::GitRepoHandle, command::run_git_output, repository_git::RepositoryGit,
        request_refs::REQUEST_REF_COMMIT_ERROR, storage::receive_pack_staging_repo_path,
    },
    persistence::ensure_private_dir,
    state::AppState,
};
use scope_domain::repository::{RepoLifecycleState, RepositoryIncarnation};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_postgres::db::GitPushContext;
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
    create_receive_pack_staging_repo(&repo_root, None)?;
    install_first_push_pre_receive_hook(&repo_root)?;
    Ok(repo_root)
}

pub(crate) struct ReceivePackStagingRepo {
    pub(crate) path: PathBuf,
    pub(crate) seed: GitRepoHandle,
}

pub(crate) async fn ensure_ready_receive_pack_staging_repo(
    state: &AppState,
    repo: &GitPushContext,
) -> Result<ReceivePackStagingRepo, ApiError> {
    if repo.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::conflict("repo must be ready before push"));
    }
    let incarnation = &repo.incarnation;
    let seed = if let Some(head) = repo.git_head.as_ref() {
        state
            .repository_engine
            .materialize_repository(state, incarnation, head, &repo.git_pack_spans)
            .await?
    } else {
        RepositoryGit::of_push_context(repo)
            .view_repo(state, repo.repo_config.views(), &repo.access.view)
            .await?
    };
    let known_head = repo.git_head.as_ref().map(|head| head.head_oid.clone());
    let state = state.clone();
    let incarnation = incarnation.clone();
    crate::git::blocking::run(move || {
        let repo_root = staging_repo_from_seed(&state, &incarnation, &seed, known_head)?;
        if let Err(error) = install_ready_pre_receive_hook(&repo_root) {
            let _ = fs::remove_dir_all(&repo_root);
            return Err(error);
        }
        Ok(ReceivePackStagingRepo {
            path: repo_root,
            seed,
        })
    })
    .await
}

struct StagingSeed<'a> {
    repo: &'a FsPath,
    main_head: Option<&'a str>,
}

const RECEIVE_PACK_STAGING_CONFIG: &str = "[core]\n\trepositoryformatversion = 0\n\tfilemode = true\n\tbare = true\n[http]\n\treceivepack = true\n[receive]\n\tunpackLimit = 1\n";

fn create_receive_pack_staging_repo(
    repo_root: &FsPath,
    seed: Option<StagingSeed<'_>>,
) -> Result<(), ApiError> {
    fs::create_dir(repo_root).map_err(ApiError::internal)?;
    for directory in [
        "objects/pack",
        "objects/info",
        "refs/heads",
        "refs/tags",
        "hooks",
    ] {
        fs::create_dir_all(repo_root.join(directory)).map_err(ApiError::internal)?;
    }
    fs::write(
        repo_root.join("HEAD"),
        format!("ref: refs/heads/{DEFAULT_GIT_BRANCH}\n"),
    )
    .map_err(ApiError::internal)?;
    fs::write(repo_root.join("config"), RECEIVE_PACK_STAGING_CONFIG).map_err(ApiError::internal)?;
    let Some(seed) = seed else {
        return Ok(());
    };
    let seed_objects = fs::canonicalize(seed.repo.join("objects")).map_err(ApiError::internal)?;
    fs::write(
        repo_root.join("objects/info/alternates"),
        format!("{}\n", seed_objects.display()),
    )
    .map_err(ApiError::internal)?;
    if let Some(main_head) = seed.main_head {
        fs::write(
            repo_root.join("refs/heads").join(DEFAULT_GIT_BRANCH),
            format!("{main_head}\n"),
        )
        .map_err(ApiError::internal)?;
    }
    Ok(())
}

pub(crate) fn create_seeded_receive_pack_staging_repo(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    seed: &FsPath,
) -> Result<PathBuf, ApiError> {
    staging_repo_from_seed(state, incarnation, seed, None)
}

fn staging_repo_from_seed(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    seed: &FsPath,
    known_head: Option<String>,
) -> Result<PathBuf, ApiError> {
    let main_head = match known_head {
        Some(head) => Some(head),
        None => seed_main_head(seed)?,
    };
    let repo_root = receive_pack_staging_repo_path(state, incarnation)?;
    if let Some(parent) = repo_root.parent() {
        ensure_private_dir(parent)?;
    }
    if let Err(error) = create_receive_pack_staging_repo(
        &repo_root,
        Some(StagingSeed {
            repo: seed,
            main_head: main_head.as_deref(),
        }),
    ) {
        let _ = fs::remove_dir_all(&repo_root);
        return Err(error);
    }
    Ok(repo_root)
}

pub(crate) fn add_receive_pack_alternate(
    repo_root: &FsPath,
    objects: &FsPath,
) -> Result<(), ApiError> {
    let objects = fs::canonicalize(objects).map_err(ApiError::internal)?;
    let alternates = repo_root.join("objects/info/alternates");
    let mut lines = fs::read_to_string(&alternates).unwrap_or_default();
    if lines.lines().any(|line| FsPath::new(line) == objects) {
        return Ok(());
    }
    lines.push_str(&format!("{}\n", objects.display()));
    fs::write(alternates, lines).map_err(ApiError::internal)
}

fn seed_main_head(seed: &FsPath) -> Result<Option<String>, ApiError> {
    let output = run_git_output(
        Some(seed),
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{DEFAULT_GIT_BRANCH}"),
        ],
        "reading the receive-pack seed head",
    )?;
    if !output.status.success() {
        return Ok(None);
    }
    String::from_utf8(output.stdout)
        .map(|head| Some(head.trim().to_string()))
        .map_err(ApiError::internal)
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
