use crate::{
    error::ApiError,
    git::{
        blocking::run as blocking,
        command::run_git,
        pool_pack::{
            MAX_INLINE_POOL_PACK_BYTES, PoolPackRevision, stage_pool_pack_revisions,
            validate_pool_pack_size, write_pool_pack_files,
        },
        request_refs::{ensure_local_bare_repo, with_request_revision_store_repo},
    },
    state::AppState,
};
use scope_domain::{
    repository::Repository,
    requests::{Request, RequestRevision},
};
use sha2::Digest as _;
use std::{collections::BTreeSet, fs, path::Path};

const USAGE: &str = "usage: api pool-pack-recovery restore OWNER/REPO BUNDLE REVISION_ID... [--write] | sweep OWNER/REPO | sweep --all";

pub async fn run(arguments: &[String]) -> anyhow::Result<()> {
    let command = Command::parse(arguments)?;
    let directory = tempfile::Builder::new()
        .prefix("scope-pool-recovery-")
        .tempdir()?;
    let database = url::Url::parse(&crate::config::database_url_from_env()?)?;
    println!(
        "database_host={}",
        database.host_str().unwrap_or("unix-socket")
    );
    let state = AppState::for_pool_pack_recovery(directory.path().to_path_buf()).await?;
    let result = match command {
        Command::Restore {
            repository,
            bundle,
            revisions,
            write,
        } => restore(&state, &repository, Path::new(&bundle), &revisions, write)
            .await
            .map(|_| ()),
        Command::Sweep { repository } => {
            sweep(&state, repository.as_deref())
                .await
                .and_then(|report| {
                    println!(
                        "scanned={} unreadable={}",
                        report.scanned,
                        report.failures.len()
                    );
                    for failure in &report.failures {
                        println!(
                            "repository={} revision={} error={}",
                            failure.repository, failure.revision, failure.error
                        );
                    }
                    if report.scanned == 0 || !report.failures.is_empty() {
                        Err(ApiError::conflict(
                            "revision sweep failed or found no revisions",
                        ))
                    } else {
                        Ok(())
                    }
                })
        }
    };
    state.shutdown().await;
    result.map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))
}

enum Command {
    Restore {
        repository: String,
        bundle: String,
        revisions: Vec<String>,
        write: bool,
    },
    Sweep {
        repository: Option<String>,
    },
}

impl Command {
    fn parse(arguments: &[String]) -> anyhow::Result<Self> {
        match arguments {
            [mode, repository] if mode == "sweep" => {
                anyhow::ensure!(repository == "--all" || valid_repository(repository), USAGE);
                Ok(Self::Sweep {
                    repository: (repository != "--all").then(|| repository.clone()),
                })
            }
            [mode, repository, bundle, revisions @ ..] if mode == "restore" => {
                let write = revisions.last().is_some_and(|arg| arg == "--write");
                let revisions = &revisions[..revisions.len() - usize::from(write)];
                anyhow::ensure!(
                    valid_repository(repository)
                        && !revisions.is_empty()
                        && revisions.iter().all(|id| !id.starts_with('-'))
                        && revisions.iter().collect::<BTreeSet<_>>().len() == revisions.len(),
                    USAGE
                );
                Ok(Self::Restore {
                    repository: repository.clone(),
                    bundle: bundle.clone(),
                    revisions: revisions.to_vec(),
                    write,
                })
            }
            _ => anyhow::bail!(USAGE),
        }
    }
}

fn valid_repository(id: &str) -> bool {
    id.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty() && !name.is_empty() && !name.contains('/') && !owner.starts_with('-')
    })
}

async fn repository(state: &AppState, id: &str) -> Result<Repository, ApiError> {
    let (owner, name) = id
        .split_once('/')
        .ok_or_else(|| ApiError::bad_request("expected OWNER/REPO"))?;
    state
        .metadata
        .repositories()
        .repository(owner, name)
        .await?
        .ok_or_else(|| ApiError::not_found("recovery repository missing"))
}

async fn read_revision(
    state: &AppState,
    repo: &Repository,
    request: &Request,
    revision: &RequestRevision,
) -> Result<(), ApiError> {
    let directory = tempfile::Builder::new()
        .prefix("scope-revision-verification-")
        .tempdir()
        .map_err(ApiError::internal)?;
    let isolated = state.with_recovery_cache(directory.path().to_path_buf())?;
    with_request_revision_store_repo(
        &isolated,
        &repo.incarnation(),
        request,
        revision,
        |path, revision| verify_connectivity(path, &revision.comparison_oids()),
    )
    .await
}

fn verify_connectivity(path: &Path, heads: &[String]) -> Result<(), ApiError> {
    let mut arguments = vec!["fsck", "--connectivity-only", "--no-dangling"];
    arguments.extend(heads.iter().map(String::as_str));
    run_git(
        Some(path),
        &arguments,
        "verify revision comparison connectivity",
    )
}

#[derive(Debug)]
struct SweepFailure {
    repository: String,
    revision: String,
    error: String,
}

#[derive(Debug, Default)]
struct SweepReport {
    scanned: usize,
    failures: Vec<SweepFailure>,
}

async fn sweep(state: &AppState, repo_id: Option<&str>) -> Result<SweepReport, ApiError> {
    if let Some(id) = repo_id {
        repository(state, id).await?;
    }
    let mut report = SweepReport::default();
    let mut after = None;
    loop {
        let batch = state
            .metadata
            .requests()
            .revision_pool_recovery_batch(repo_id, after.as_deref())
            .await?;
        if batch.is_empty() {
            break;
        }
        for revision in batch {
            after = Some(revision.id.clone());
            let request = state
                .metadata
                .requests()
                .request_by_id(&revision.request_id)
                .await?
                .ok_or_else(|| ApiError::not_found("sweep request missing"))?;
            report.scanned += 1;
            let read = async {
                let repo = repository(state, &request.repo_id).await?;
                read_revision(state, &repo, &request, &revision).await
            }
            .await;
            if let Err(error) = read {
                report.failures.push(SweepFailure {
                    repository: request.repo_id,
                    revision: revision.id,
                    error: error.into_operator_diagnostic(),
                });
            }
        }
    }
    Ok(report)
}

#[derive(Debug, PartialEq, Eq)]
enum RestoreOutcome {
    AlreadyReadable,
    WouldRestore,
    Restored,
}

async fn restore(
    state: &AppState,
    repo_id: &str,
    bundle: &Path,
    revision_ids: &[String],
    write: bool,
) -> Result<Vec<RestoreOutcome>, ApiError> {
    let repo = repository(state, repo_id).await?;
    let mut selected = Vec::new();
    let mut outcomes = Vec::new();
    for id in revision_ids {
        let revision = state
            .metadata
            .requests()
            .revision_for_pool_recovery(id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("revision {id} missing")))?;
        let request = state
            .metadata
            .requests()
            .request_by_id(&revision.request_id)
            .await?
            .filter(|request| request.repo_id == repo_id)
            .ok_or_else(|| ApiError::conflict(format!("revision {id} is not in {repo_id}")))?;
        let owner = revision.restoration_pack_owner(&request, repo.repo_config.views())?;
        if read_revision(state, &repo, &request, &revision)
            .await
            .is_ok()
        {
            println!(
                "repository={repo_id} revision={} already_readable",
                revision.id
            );
            outcomes.push(RestoreOutcome::AlreadyReadable);
            continue;
        }
        selected.push((request, revision, owner));
    }
    if selected.is_empty() {
        return Ok(outcomes);
    }
    let bundle = fs::canonicalize(bundle).map_err(ApiError::internal)?;
    let directory = tempfile::Builder::new()
        .prefix("scope-recovery-bundle-")
        .tempdir()
        .map_err(ApiError::internal)?;
    let source = directory.path().join("source.git");
    let source = blocking(move || {
        ensure_local_bare_repo(source.clone())?;
        let bundle = bundle
            .to_str()
            .ok_or_else(|| ApiError::bad_request("bundle path must be UTF-8"))?;
        run_git(
            Some(&source),
            &["bundle", "verify", bundle],
            "verify recovery bundle",
        )?;
        run_git(
            Some(&source),
            &["bundle", "unbundle", bundle],
            "import recovery bundle",
        )?;
        Ok(source)
    })
    .await?;
    for (_, revision, _) in &selected {
        let source = source.clone();
        let heads = revision.comparison_oids();
        blocking(move || verify_connectivity(&source, &heads)).await?;
    }
    for (request, revision, owner) in selected {
        let pack_source = source.clone();
        let heads = revision.comparison_oids();
        let bytes = blocking(move || {
            let files = write_pool_pack_files(&pack_source, &heads)?;
            let verification = tempfile::tempdir().map_err(ApiError::internal)?;
            let target = ensure_local_bare_repo(verification.path().join("verify.git"))?;
            for (file, extension) in [(&files.pack, "pack"), (&files.index, "idx")] {
                fs::copy(
                    file,
                    target.join(format!(
                        "objects/pack/pack-{}.{}",
                        files.checksum, extension
                    )),
                )
                .map_err(ApiError::internal)?;
            }
            verify_connectivity(&target, &heads)?;
            Ok(fs::metadata(files.pack).map_err(ApiError::internal)?.len())
        })
        .await?;
        validate_pool_pack_size(state, bytes)?;
        let pack_id = recovery_pack_id(&repo, &revision)?;
        let storage = if bytes <= MAX_INLINE_POOL_PACK_BYTES {
            "inline"
        } else {
            "segment"
        };
        println!(
            "repository={repo_id} revision={} head={} scope={:?} pack_id={pack_id} prerequisites=0 pack_bytes={bytes} storage={storage} write={write}",
            revision.id, revision.new_head_oid, owner
        );
        if !write {
            outcomes.push(RestoreOutcome::WouldRestore);
            continue;
        }
        let staged = stage_pool_pack_revisions(
            state,
            repo_id,
            &source,
            PoolPackRevision {
                owner,
                head_oid: revision.new_head_oid.clone(),
                prerequisites: Vec::new(),
            },
            revision.comparison_oids(),
            Some(pack_id),
        )
        .await?;
        let registration = state
            .metadata
            .repositories()
            .restore_revision_pool_pack(
                &repo,
                &request,
                &revision,
                &staged.pack,
                crate::persistence::unix_now()?,
            )
            .await;
        match registration {
            Ok(true) => staged.finish(state).await,
            Ok(false) => staged.discard(state, repo_id).await,
            Err(error) => {
                staged.discard(state, repo_id).await;
                return Err(error.into());
            }
        }
        read_revision(state, &repo, &request, &revision).await?;
        println!(
            "repository={repo_id} revision={} verified_from_empty_shelf",
            revision.id
        );
        outcomes.push(RestoreOutcome::Restored);
    }
    Ok(outcomes)
}

fn recovery_pack_id(repo: &Repository, revision: &RequestRevision) -> Result<String, ApiError> {
    let identity = serde_json::to_vec(&(
        repo.incarnation(),
        repo.repo_config.object_generation(),
        revision,
    ))
    .map_err(ApiError::internal)?;
    Ok(format!(
        "git_pool_pack_restore_{}",
        hex::encode(sha2::Sha256::digest(identity))
    ))
}

#[cfg(test)]
mod tests;
