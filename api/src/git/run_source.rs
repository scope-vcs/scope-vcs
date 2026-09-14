pub(super) mod operation;

use crate::{
    error::ApiError,
    git::{
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::{git_process_output, truncated_git_stderr},
        repository_engine::GitRevision,
    },
    runtime_budgets::RuntimePermit,
    state::AppState,
};
use axum::body::{Body, Bytes};
use scope_domain::{
    repository::RepositoryIncarnation,
    runs::{run::Run, source::RunSource, workflow::identity::WorkflowPath},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, run as run_process};
use scope_object_store::source_blob_bytes_bounded;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    os::unix::{fs::DirBuilderExt as _, fs::OpenOptionsExt as _},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tokio::io::AsyncReadExt as _;

const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);
const RUN_SOURCE_STREAM_CHUNK_BYTES: usize = 64 * 1024;

pub(crate) struct MaterializedRunSource {
    pub(crate) sha256: String,
    body: RunSourceBody,
}

enum RunSourceBody {
    Buffered(Vec<u8>),
    /// Streams the cached bundle from disk. The cache lease and the object-store
    /// permit live as long as the stream, so eviction and the read budget both
    /// see the download until its last byte.
    Cached {
        file: tokio::fs::File,
        length: u64,
        handle: GitRepoHandle,
        permit: RuntimePermit,
    },
}

impl MaterializedRunSource {
    pub(crate) fn content_length(&self) -> u64 {
        match &self.body {
            RunSourceBody::Buffered(bytes) => bytes.len() as u64,
            RunSourceBody::Cached { length, .. } => *length,
        }
    }

    pub(crate) fn into_body(self) -> Body {
        match self.body {
            RunSourceBody::Buffered(bytes) => Body::from(bytes),
            RunSourceBody::Cached {
                file,
                handle,
                permit,
                ..
            } => Body::from_stream(futures_util::stream::try_unfold(
                (file, handle, permit),
                |(mut file, handle, permit)| async move {
                    let mut chunk = vec![0_u8; RUN_SOURCE_STREAM_CHUNK_BYTES];
                    let read = file.read(&mut chunk).await?;
                    if read == 0 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    chunk.truncate(read);
                    Ok(Some((Bytes::from(chunk), (file, handle, permit))))
                },
            )),
        }
    }
}

pub(crate) async fn materialize_run_source_bundle(
    state: &AppState,
    run: &Run,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let source = &run.source;
    if let Some(bundle) = source.ephemeral_bundle() {
        let object_store = state.object_store.clone();
        let bundle = bundle.clone();
        return tokio::task::spawn_blocking(move || {
            let bytes = source_blob_bytes_bounded(object_store.as_ref(), &bundle, max_bytes)
                .map_err(ApiError::from)?;
            Ok(MaterializedRunSource {
                sha256: hex::encode(Sha256::digest(&bytes)),
                body: RunSourceBody::Buffered(bytes),
            })
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source object read task failed: {error}"))
        })?;
    }
    let incarnation = state
        .metadata
        .repositories()
        .run_repository_incarnation(&run.id, run.workflow.repository_id())
        .await?
        .ok_or_else(|| ApiError::not_found("run repository not found"))?;
    materialize_accepted_git_head_bundle(state, &incarnation, source, max_bytes).await
}

fn run_source_cache_key(
    incarnation: &RepositoryIncarnation,
    source: &RunSource,
) -> Result<String, ApiError> {
    let RunSource::AcceptedGitHead { head, audience, .. } = source else {
        return Err(ApiError::internal_message(
            "run bundle cache requires an accepted Git head",
        ));
    };
    let identity = serde_json::to_vec(&(
        "run-source-bundle",
        incarnation.repository_id(),
        incarnation.incarnation_id(),
        &head.head_oid,
        audience,
    ))
    .map_err(ApiError::internal)?;
    Ok(format!(
        "run-source-{}",
        hex::encode(Sha256::digest(identity))
    ))
}

async fn materialize_accepted_git_head_bundle(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    source: &RunSource,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let key = run_source_cache_key(incarnation, source)?;
    let path = state
        .repository_engine
        .cache_root()
        .join(format!("{key}.git"));
    let ready_path = path.clone();
    let build_path = path.clone();
    let build_state = state.clone();
    let build_source = source.clone();
    let build_incarnation = incarnation.clone();
    let handle = state
        .repository_engine
        .materialize_derived(
            incarnation,
            GitDerivedCacheNamespace::RunSource,
            key,
            &path,
            move || {
                ready_path.join("source.bundle").is_file() && ready_path.join("sha256").is_file()
            },
            move || {
                build_accepted_source_bundle(
                    build_state,
                    build_incarnation,
                    build_source,
                    build_path,
                    max_bytes,
                )
            },
        )
        .await?;
    let permit = state
        .runtime_budgets
        .try_object_store("run source cache read")?;
    let file = tokio::fs::File::open(handle.join("source.bundle"))
        .await
        .map_err(ApiError::internal)?;
    let length = file.metadata().await.map_err(ApiError::internal)?.len();
    if length > max_bytes as u64 {
        return Err(ApiError::bad_request(format!(
            "Git command output exceeds {max_bytes} bytes"
        )));
    }
    let sha256 = tokio::fs::read(handle.join("sha256"))
        .await
        .map_err(ApiError::internal)?;
    if sha256.len() != 64 {
        return Err(ApiError::internal_message(
            "run source cache digest is malformed",
        ));
    }
    let sha256 = String::from_utf8(sha256).map_err(ApiError::internal)?;
    Ok(MaterializedRunSource {
        sha256,
        body: RunSourceBody::Cached {
            file,
            length,
            handle,
            permit,
        },
    })
}

async fn build_accepted_source_bundle(
    state: AppState,
    incarnation: RepositoryIncarnation,
    source: RunSource,
    path: PathBuf,
    max_bytes: usize,
) -> Result<(), ApiError> {
    operation::supervise(async move {
        let (_, head, pack_spans) = source.logical_git_head().ok_or_else(|| {
            ApiError::internal_message("run source does not contain a materializable Git head")
        })?;
        let revision = state
            .repository_engine
            .materialize_revision(&state, &incarnation, head, pack_spans)
            .await?;
        let owner = operation::RunSourceOperation::new(&state)?;
        let bytes =
            materialize_owned_git_head_bundle(&state, revision, max_bytes, owner.clone()).await?;
        let temporary = TemporarySourceDirectory::new(state.repository_engine.cache_root())?;
        operation::spawn_blocking(&owner, move || {
            write_private_file(&temporary.path.join("source.bundle"), &bytes)?;
            write_private_file(
                &temporary.path.join("sha256"),
                hex::encode(Sha256::digest(&bytes)).as_bytes(),
            )?;
            fs::rename(&temporary.path, path).map_err(ApiError::internal)
        })
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source cache publication task failed: {error}"))
        })?
    })
    .await
}

async fn materialize_owned_git_head_bundle(
    state: &AppState,
    revision: GitRevision,
    max_bytes: usize,
    owner: std::sync::Arc<operation::RunSourceOperation>,
) -> Result<Vec<u8>, ApiError> {
    let repo = operation::repository(&owner);
    let revision = operation::spawn_blocking(&owner, move || {
        revision.create_view(&repo)?;
        Ok::<_, ApiError>(revision)
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run source revision task failed: {error}"))
    })??;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let repo_path = operation::repository(&owner);
    let timeout = state.runtime_budgets.git_command_timeout();
    let output = operation::spawn_blocking(&owner, move || {
        let _revision = revision;
        git_process_output(
            Command::new("git")
                .arg("--git-dir")
                .arg(repo_path)
                .args(["bundle", "create", "-", &main_ref]),
            None,
            ProcessLimits::new(timeout).with_max_stdout_bytes(max_bytes),
        )
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!("run source Git bundle task failed: {error}"))
    })??;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "materializing run source bundle: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub(crate) fn inspect_manual_run_bundle(
    root: &Path,
    bytes: &[u8],
    git_oid: &str,
    workflow_name: &str,
) -> Result<scope_run_config::ParsedWorkflow, ApiError> {
    let yml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yml"))
        .map_err(ApiError::bad_request)?;
    let yaml = WorkflowPath::parse(format!("/.scope/runs/{workflow_name}.yaml"))
        .map_err(ApiError::bad_request)?;
    let temp = TemporarySourceDirectory::new(root)?;
    let bundle = temp.path.join("source.bundle");
    let bare = temp.path.join("source.git");
    write_private_file(&bundle, bytes)?;
    let mut clone = Command::new("git");
    clone
        .args(["clone", "--bare", "--no-local"])
        .arg(&bundle)
        .arg(&bare);
    if run_git_inspection(&mut clone, "Git bundle clone", 0)?.is_none() {
        return Err(ApiError::bad_request("invalid Git bundle"));
    }
    let mut commit = Command::new("git");
    commit
        .arg("--git-dir")
        .arg(&bare)
        .args(["cat-file", "-e", &format!("{git_oid}^{{commit}}")]);
    if run_git_inspection(&mut commit, "Git commit inspection", 0)?.is_none() {
        return Err(ApiError::bad_request(
            "requested Git commit is not present in the bundle",
        ));
    }
    let yml_bytes = git_blob(&bare, git_oid, yml.as_str().trim_start_matches('/'))?;
    let yaml_bytes = git_blob(&bare, git_oid, yaml.as_str().trim_start_matches('/'))?;
    let (path, workflow_bytes) = match (yml_bytes, yaml_bytes) {
        (Some(_), Some(_)) => {
            return Err(ApiError::bad_request(format!(
                "workflow {workflow_name:?} is defined by both .yml and .yaml"
            )));
        }
        (Some(bytes), None) => (yml, bytes),
        (None, Some(bytes)) => (yaml, bytes),
        (None, None) => {
            return Err(ApiError::not_found(format!(
                "workflow {workflow_name:?} was not found at commit {git_oid}"
            )));
        }
    };
    scope_run_config::parse_workflow(path.as_str(), &workflow_bytes).map_err(ApiError::bad_request)
}

fn git_blob(bare: &Path, git_oid: &str, path: &str) -> Result<Option<Vec<u8>>, ApiError> {
    let object = format!("{git_oid}:{path}");
    let mut inspect = Command::new("git");
    inspect
        .arg("--git-dir")
        .arg(bare)
        .args(["ls-tree", "-lz", git_oid, "--", path]);
    let output = git_process_output(
        &mut inspect,
        None,
        ProcessLimits::new(GIT_INSPECTION_TIMEOUT).with_max_stdout_bytes(path.len() + 128),
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading Git workflow metadata: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    if output.stdout.is_empty() {
        return Ok(None);
    }
    let entry = std::str::from_utf8(&output.stdout)
        .map_err(|_| ApiError::infrastructure_unavailable("invalid Git workflow metadata"))?
        .trim_end_matches('\0');
    let Some((metadata, listed_path)) = entry.split_once('\t') else {
        return Err(ApiError::infrastructure_unavailable(
            "invalid Git workflow metadata",
        ));
    };
    if listed_path != path {
        return Err(ApiError::infrastructure_unavailable(
            "Git returned a different workflow path",
        ));
    }
    let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
    let [_, kind, _, size] = fields.as_slice() else {
        return Err(ApiError::infrastructure_unavailable(
            "invalid Git workflow metadata",
        ));
    };
    if *kind != "blob" {
        return Err(ApiError::bad_request(
            "workflow definition must be a Git blob",
        ));
    }
    let size = size
        .parse::<usize>()
        .map_err(|_| ApiError::infrastructure_unavailable("invalid Git workflow size"))?;
    if size > scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES {
        return Err(ApiError::bad_request(format!(
            "workflow definition exceeds {} bytes",
            scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES
        )));
    }

    let mut blob = Command::new("git");
    blob.arg("--git-dir")
        .arg(bare)
        .args(["cat-file", "blob", &object]);
    let output = git_process_output(
        &mut blob,
        None,
        ProcessLimits::new(GIT_INSPECTION_TIMEOUT)
            .with_max_stdout_bytes(scope_run_config::MAX_WORKFLOW_DEFINITION_BYTES),
    )?;
    if !output.status.success() || output.stdout.len() != size {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading Git workflow blob failed: {}",
            truncated_git_stderr(&output.stderr).trim()
        )));
    }
    Ok(Some(output.stdout))
}

/// Runs an inspection command against caller-supplied bundle content. Timeouts and
/// oversized output are the bundle's fault, so they surface as bad requests; a non-zero
/// exit yields `None` for the caller to interpret.
fn run_git_inspection(
    command: &mut Command,
    operation: &str,
    max_stdout_bytes: usize,
) -> Result<Option<Vec<u8>>, ApiError> {
    let output = run_process(
        command,
        None,
        ProcessLimits::new(GIT_INSPECTION_TIMEOUT).with_max_stdout_bytes(max_stdout_bytes),
        operation,
    )
    .map_err(|error| {
        if error.is_timeout() || error.is_stdout_limit() {
            ApiError::bad_request(error.to_string())
        } else {
            ApiError::internal_message(error.to_string())
        }
    })?;
    Ok(output.status.success().then_some(output.stdout))
}

fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let mut file = create_private_file(path)?;
    std::io::Write::write_all(&mut file, bytes).map_err(ApiError::internal)
}

fn create_private_file(path: &Path) -> Result<File, ApiError> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(ApiError::internal)
}

struct TemporarySourceDirectory {
    path: PathBuf,
}

impl TemporarySourceDirectory {
    fn path(&self) -> &Path {
        &self.path
    }

    fn new(root: &Path) -> Result<Self, ApiError> {
        fs::create_dir_all(root).map_err(ApiError::internal)?;
        for _ in 0..8 {
            let path = root.join(format!(
                "{}.tmp",
                crate::persistence_ids::generate_prefixed_id("inspect")?
            ));
            match fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(ApiError::internal(error)),
            }
        }
        Err(ApiError::internal_message(
            "could not allocate run bundle inspection directory",
        ))
    }
}

impl Drop for TemporarySourceDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests;
