pub(super) mod operation;

use crate::{
    error::ApiError,
    git::{
        cache::{GitDerivedCacheNamespace, GitRepoHandle},
        command::{
            git_process_output, git_stdout_text, run_git, run_git_output, truncated_git_stderr,
        },
        repository_engine::GitRevision,
    },
    state::AppState,
};
use axum::body::{Body, Bytes};
use scope_domain::{
    repository::RepositoryIncarnation,
    runs::{run::Run, source::RunSource, workflow::identity::WorkflowPath},
};
use scope_git::DEFAULT_GIT_BRANCH;
use scope_git_process::{ProcessLimits, run as run_process};
use scope_storage::source_blob_bytes;
use sha2::{Digest as _, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    future::Future,
    os::unix::{fs::DirBuilderExt as _, fs::OpenOptionsExt as _},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tokio::io::AsyncReadExt as _;

const GIT_INSPECTION_TIMEOUT: Duration = Duration::from_secs(30);
const RUN_SOURCE_STREAM_CHUNK_BYTES: usize = 64 * 1024;
const RUN_SOURCE_DIGEST_BYTES: u64 = 64;

pub(crate) struct MaterializedRunSource {
    pub(crate) sha256: String,
    body: RunSourceBody,
}

enum RunSourceBody {
    Buffered(Vec<u8>),
    /// Streams the cached bundle from disk. The cache lease lives as long as the
    /// stream, so eviction sees the download until its last byte. The object-store
    /// permit does not: the stream reads a local file, so holding it would let
    /// slow clients starve real object-store operations.
    Cached {
        file: tokio::fs::File,
        length: u64,
        handle: GitRepoHandle,
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
            RunSourceBody::Cached { file, handle, .. } => Body::from_stream(
                futures_util::stream::try_unfold((file, handle), |(mut file, handle)| async move {
                    let mut chunk = vec![0_u8; RUN_SOURCE_STREAM_CHUNK_BYTES];
                    let read = file.read(&mut chunk).await?;
                    if read == 0 {
                        return Ok::<_, std::io::Error>(None);
                    }
                    chunk.truncate(read);
                    Ok(Some((Bytes::from(chunk), (file, handle))))
                }),
            ),
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
        // The read checks the bundle against its recorded digest.
        let bytes = source_blob_bytes(state.object_store.as_ref(), bundle, max_bytes).await?;
        return Ok(MaterializedRunSource {
            sha256: bundle.sha256.clone(),
            body: RunSourceBody::Buffered(bytes),
        });
    }
    let incarnation = state
        .metadata
        .repositories()
        .run_repository_incarnation(&run.id, run.workflow.repository_id())
        .await?
        .ok_or_else(|| ApiError::not_found("run repository not found"))?;
    if let Some((bundle, base_oid)) = source.request_git_source() {
        return materialize_request_git_bundle(state, &incarnation, bundle, base_oid, max_bytes)
            .await;
    }
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

fn request_source_cache_key(
    incarnation: &RepositoryIncarnation,
    bundle: &scope_domain::content::SourceBlob,
    base_oid: &str,
) -> Result<String, ApiError> {
    let identity = serde_json::to_vec(&(
        "standalone-request-run-source-v1",
        incarnation,
        &bundle.sha256,
        &bundle.git_oid,
        base_oid,
    ))
    .map_err(ApiError::internal)?;
    Ok(format!(
        "run-source-{}",
        hex::encode(Sha256::digest(identity))
    ))
}

async fn materialize_request_git_bundle(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    snapshot: &scope_domain::content::SourceBlob,
    base_oid: &str,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let key = request_source_cache_key(incarnation, snapshot, base_oid)?;
    let build_state = state.clone();
    let build_incarnation = incarnation.clone();
    let snapshot = snapshot.clone();
    let base_oid = base_oid.to_string();
    cached_run_source_bundle(state, incarnation, key, max_bytes, move |path| {
        build_request_source_bundle(
            build_state,
            build_incarnation,
            snapshot,
            base_oid,
            path,
            max_bytes,
        )
    })
    .await
}

async fn materialize_accepted_git_head_bundle(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    source: &RunSource,
    max_bytes: usize,
) -> Result<MaterializedRunSource, ApiError> {
    let key = run_source_cache_key(incarnation, source)?;
    let build_state = state.clone();
    let build_source = source.clone();
    let build_incarnation = incarnation.clone();
    cached_run_source_bundle(state, incarnation, key, max_bytes, move |path| {
        build_accepted_source_bundle(
            build_state,
            build_incarnation,
            build_source,
            path,
            max_bytes,
        )
    })
    .await
}

async fn cached_run_source_bundle<Build, BuildFuture>(
    state: &AppState,
    incarnation: &RepositoryIncarnation,
    key: String,
    max_bytes: usize,
    build: Build,
) -> Result<MaterializedRunSource, ApiError>
where
    Build: FnOnce(PathBuf) -> BuildFuture + Send + 'static,
    BuildFuture: Future<Output = Result<(), ApiError>> + Send + 'static,
{
    let path = state
        .repository_engine
        .cache_root()
        .join(format!("{key}.git"));
    let ready_path = path.clone();
    let build_path = path.clone();
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
            move || build(build_path),
        )
        .await?;
    // The permit covers opening the cache entry, not the client's download.
    let _permit = state
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
    let sha256 = read_cached_digest(&handle.join("sha256")).await?;
    Ok(MaterializedRunSource {
        sha256,
        body: RunSourceBody::Cached {
            file,
            length,
            handle,
        },
    })
}

async fn read_cached_digest(path: &Path) -> Result<String, ApiError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(ApiError::internal)?;
    if file.metadata().await.map_err(ApiError::internal)?.len() != RUN_SOURCE_DIGEST_BYTES {
        return Err(ApiError::internal_message(
            "run source cache digest is malformed",
        ));
    }
    let mut digest = Vec::with_capacity(RUN_SOURCE_DIGEST_BYTES as usize);
    file.take(RUN_SOURCE_DIGEST_BYTES)
        .read_to_end(&mut digest)
        .await
        .map_err(ApiError::internal)?;
    String::from_utf8(digest).map_err(ApiError::internal)
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
        let bytes = materialize_owned_git_head_bundle(
            &state,
            BundleView::Accepted(revision),
            max_bytes,
            owner.clone(),
        )
        .await?;
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

async fn build_request_source_bundle(
    state: AppState,
    incarnation: RepositoryIncarnation,
    snapshot: scope_domain::content::SourceBlob,
    base_oid: String,
    path: PathBuf,
    max_bytes: usize,
) -> Result<(), ApiError> {
    operation::supervise(async move {
        let (main_head, spans) = state
            .metadata
            .repositories()
            .repository_content_source(&incarnation)
            .await?;
        let revision = if let Some(main_head) = main_head {
            Some(
                state
                    .repository_engine
                    .materialize_revision(&state, &incarnation, &main_head, &spans)
                    .await?,
            )
        } else {
            // Before the first Git push, a private request starts from the
            // private projection. Its snapshot already has complete history.
            None
        };
        let bytes = source_blob_bytes(state.object_store.as_ref(), &snapshot, max_bytes).await?;
        let owner = operation::RunSourceOperation::new(&state)?;
        let bundle = materialize_owned_git_head_bundle(
            &state,
            BundleView::Request {
                base_revision: revision,
                snapshot: RequestSnapshot {
                    bytes,
                    base_oid,
                    head_oid: snapshot.git_oid,
                },
            },
            max_bytes,
            owner.clone(),
        )
        .await?;
        let temporary = TemporarySourceDirectory::new(state.repository_engine.cache_root())?;
        operation::spawn_blocking(&owner, move || {
            write_private_file(&temporary.path.join("source.bundle"), &bundle)?;
            write_private_file(
                &temporary.path.join("sha256"),
                hex::encode(Sha256::digest(&bundle)).as_bytes(),
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

struct RequestSnapshot {
    bytes: Vec<u8>,
    base_oid: String,
    head_oid: String,
}

enum BundleView {
    Accepted(GitRevision),
    Request {
        base_revision: Option<GitRevision>,
        snapshot: RequestSnapshot,
    },
}

async fn materialize_owned_git_head_bundle(
    state: &AppState,
    view: BundleView,
    max_bytes: usize,
    owner: std::sync::Arc<operation::RunSourceOperation>,
) -> Result<Vec<u8>, ApiError> {
    let repo = operation::repository(&owner);
    let verify_standalone = matches!(view, BundleView::Request { .. });
    let revision = operation::spawn_blocking(&owner, move || prepare_bundle_view(&repo, view))
        .await
        .map_err(|error| {
            ApiError::internal_message(format!("run source revision task failed: {error}"))
        })??;
    let main_ref = format!("refs/heads/{DEFAULT_GIT_BRANCH}");
    let repo_path = operation::repository(&owner);
    let verify_repo = repo_path.clone();
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
        .map_err(|error| {
            if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
                ApiError::payload_too_large(format!("run source bundle exceeds {max_bytes} bytes"))
            } else {
                error
            }
        })
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
    let bytes = output.stdout;
    if !verify_standalone {
        return Ok(bytes);
    }
    operation::spawn_blocking(&owner, move || {
        verify_standalone_bundle(&verify_repo, &bytes)?;
        Ok::<_, ApiError>(bytes)
    })
    .await
    .map_err(|error| {
        ApiError::internal_message(format!(
            "run source bundle verification task failed: {error}"
        ))
    })?
}

fn prepare_bundle_view(repo: &Path, view: BundleView) -> Result<Option<GitRevision>, ApiError> {
    match view {
        BundleView::Accepted(revision) => {
            revision.create_view(repo)?;
            Ok(Some(revision))
        }
        BundleView::Request {
            base_revision,
            snapshot,
        } => {
            run_git(
                None,
                &["init", "--bare", repo.to_string_lossy().as_ref()],
                "initializing request run source",
            )?;
            let bundle = repo.join("request.bundle");
            write_private_file(&bundle, &snapshot.bytes)?;
            let standalone = run_git_output(
                Some(repo),
                &["bundle", "verify", bundle.to_string_lossy().as_ref()],
                "checking request snapshot prerequisites",
            )?;
            if !standalone.status.success() {
                let revision = base_revision.as_ref().ok_or_else(|| {
                    ApiError::infrastructure_unavailable(format!(
                        "request snapshot needs an unavailable base: {}",
                        truncated_git_stderr(&standalone.stderr).trim()
                    ))
                })?;
                revision.create_view(repo)?;
                let base = format!("{}^{{commit}}", snapshot.base_oid);
                run_git(
                    Some(repo),
                    &["cat-file", "-e", &base],
                    "checking request run source base",
                )?;
            }
            let refspec = format!("+{}:refs/heads/{DEFAULT_GIT_BRANCH}", snapshot.head_oid);
            run_git(
                Some(repo),
                &[
                    "fetch",
                    "--no-tags",
                    bundle.to_string_lossy().as_ref(),
                    &refspec,
                ],
                "restoring request run source against its pinned base",
            )?;
            fs::remove_file(&bundle).map_err(ApiError::internal)?;
            Ok(base_revision)
        }
    }
}

fn verify_standalone_bundle(repo: &Path, bytes: &[u8]) -> Result<(), ApiError> {
    let source = repo.join("published.bundle");
    let clone = repo.join("standalone.git");
    write_private_file(&source, bytes)?;
    run_git(
        None,
        &[
            "clone",
            "--bare",
            "--no-local",
            source.to_string_lossy().as_ref(),
            clone.to_string_lossy().as_ref(),
        ],
        "verifying standalone run source bundle",
    )?;
    let expected = git_stdout_text(
        repo,
        &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
        "reading run source head",
    )?;
    let actual = git_stdout_text(
        &clone,
        &["rev-parse", &format!("refs/heads/{DEFAULT_GIT_BRANCH}")],
        "reading standalone run source head",
    )?;
    if expected != actual {
        return Err(ApiError::infrastructure_unavailable(
            "standalone run source does not match its pinned head",
        ));
    }
    Ok(())
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
