use super::pushed_tree::{
    GitTreeFile, git_changed_tree_entries, git_tree_entries_under, validate_pushed_commit_range,
};
use super::repo_io::{
    PushedCommit, StagedGitPush, describe_refs, git_push_from_repo, git_refs, pushed_commit,
};
use super::staging::{ReceivePackFileChange, ReceivePackUpdate, ensure_default_branch};
use crate::{
    error::ApiError,
    git::{
        command::{git_process_output, run_git_output, run_git_output_bounded},
        pool_pack::StagedPoolPack,
    },
    runtime_budgets::RuntimeBudgets,
    state::AppState,
};
use scope_domain::landing_file::{
    MAX_REPOSITORY_LANDING_FILE_BYTES, REPOSITORY_LANDING_FILE_PATH, RepositoryLandingFile,
    RepositoryLandingFileMutation,
};
use scope_domain::policy::ScopePath;
use scope_domain::repo_config::RepoConfig;
use scope_domain::repository::{RepoLifecycleState, Repository, git::GitHead};
use scope_domain::runs::{
    catalog::{
        MAX_REPOSITORY_WORKFLOW_FILES, MAX_WORKFLOW_DEFINITION_BYTES, RepositoryWorkflowCatalog,
        RepositoryWorkflowFile,
    },
    workflow::identity::WorkflowPath,
};
use scope_git::git_blob_reference;
use scope_git_process::ProcessLimits;
use std::{path::Path as FsPath, process::Command, time::Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReviewedUpdateMode {
    FirstPush,
    ReadyPush,
    RequestMerge,
}

pub(crate) struct PreparedReceivePackUpdate {
    pub(crate) update: ReceivePackUpdate,
    pub(crate) staged_pack: StagedPoolPack,
}

impl std::fmt::Debug for PreparedReceivePackUpdate {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PreparedReceivePackUpdate")
            .field("update", &self.update)
            .finish_non_exhaustive()
    }
}

impl std::ops::Deref for PreparedReceivePackUpdate {
    type Target = ReceivePackUpdate;

    fn deref(&self) -> &Self::Target {
        &self.update
    }
}

impl std::ops::DerefMut for PreparedReceivePackUpdate {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.update
    }
}

pub(crate) async fn reviewed_update_from_staging_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
    staging_repo: &FsPath,
    author_id: &str,
    config: RepoConfig,
    mode: ReviewedUpdateMode,
) -> Result<PreparedReceivePackUpdate, ApiError> {
    let repo = state
        .metadata
        .repositories()
        .git_push_state(owner, repo_name, author_id)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("repo {owner}/{repo_name} not found")))?;
    reviewed_update_on_base(
        state,
        ReviewTarget {
            owner,
            repo_name,
            staging_repo,
            author_id,
        },
        ReviewBase {
            repo_id: &repo.repo_id,
            lifecycle_state: repo.lifecycle_state,
            git_head: repo.git_head.as_ref(),
            change_version: repo.change_version,
            repo_config: &repo.repo_config,
        },
        config,
        mode,
    )
    .await
}

pub(crate) async fn reviewed_merge_update(
    state: &AppState,
    target: ReviewTarget<'_>,
    base: &Repository,
) -> Result<PreparedReceivePackUpdate, ApiError> {
    reviewed_update_on_base(
        state,
        target,
        ReviewBase {
            repo_id: &base.record.id,
            lifecycle_state: base.record.lifecycle_state,
            git_head: base.git_head.as_ref(),
            change_version: base.record.change_version,
            repo_config: &base.repo_config,
        },
        base.repo_config.clone(),
        ReviewedUpdateMode::RequestMerge,
    )
    .await
}

pub(crate) struct ReviewTarget<'a> {
    pub(crate) owner: &'a str,
    pub(crate) repo_name: &'a str,
    pub(crate) staging_repo: &'a FsPath,
    pub(crate) author_id: &'a str,
}

struct ReviewBase<'a> {
    repo_id: &'a str,
    lifecycle_state: RepoLifecycleState,
    git_head: Option<&'a GitHead>,
    change_version: u64,
    repo_config: &'a RepoConfig,
}

async fn reviewed_update_on_base(
    state: &AppState,
    target: ReviewTarget<'_>,
    repo: ReviewBase<'_>,
    config: RepoConfig,
    mode: ReviewedUpdateMode,
) -> Result<PreparedReceivePackUpdate, ApiError> {
    let ReviewTarget {
        owner,
        repo_name,
        staging_repo,
        author_id,
    } = target;
    let (branch, head_oid) = single_pushed_branch(staging_repo).await?;
    if mode != ReviewedUpdateMode::FirstPush && repo.lifecycle_state != RepoLifecycleState::Ready {
        return Err(ApiError::conflict("repo must be ready before push"));
    }
    if mode == ReviewedUpdateMode::ReadyPush
        && let Some(current_head) = repo.git_head.map(|head| head.head_oid.clone())
    {
        let path = staging_repo.to_path_buf();
        let pushed_head = head_oid.clone();
        crate::git::blocking::run(move || {
            ensure_pushed_head_extends(&path, &current_head, &pushed_head)
        })
        .await?;
    }
    let base_config_hash = crate::push_intents::repo_config_fingerprint(repo.repo_config)?;
    let inspected = {
        let path = staging_repo.to_path_buf();
        let head = head_oid.clone();
        let repo_id = repo.repo_id.to_string();
        let base_head = repo.git_head.map(|head| head.head_oid.clone());
        let change_version = repo.change_version.saturating_add(1);
        crate::git::blocking::run(move || {
            inspect_received_tree(
                &path,
                &head,
                base_head.as_deref(),
                &repo_id,
                change_version,
                mode,
            )
        })
        .await?
    };
    let InspectedReceiveTree {
        message,
        occurred_at_unix,
        changes,
        landing_file_mutation,
        workflow_catalog,
        diff_ms,
    } = inspected;
    let pack_started = Instant::now();
    let StagedGitPush {
        head: mut git_head,
        pack: staged_pack,
    } = git_push_from_repo(state, repo.repo_id, staging_repo, repo.git_head).await?;
    git_head.change_version = repo.change_version.saturating_add(1);
    let pack_put_ms = pack_started.elapsed().as_millis();
    let pack_bytes = staged_pack.pack.storage.stored_bytes();
    tracing::info!(
        owner,
        repo = repo_name,
        changed_files = changes.len(),
        pack_bytes,
        diff_ms,
        pack_put_ms,
        "prepared durable Git push objects"
    );

    Ok(PreparedReceivePackUpdate {
        update: ReceivePackUpdate {
            occurred_at_unix,
            branch,
            head_oid,
            base_git_frontier: None,
            author_id: author_id.to_string(),
            message,
            git_head,
            main_pack: staged_pack.pack.clone(),
            workflow_catalog,
            landing_file_mutation,
            changes,
            previous_config: Some(repo.repo_config.clone()),
            base_config_hash,
            config,
            open_requests_by_view: Default::default(),
        },
        staged_pack,
    })
}

async fn single_pushed_branch(staging_repo: &FsPath) -> Result<(String, String), ApiError> {
    let path = staging_repo.to_path_buf();
    let refs = crate::git::blocking::run(move || git_refs(&path)).await?;
    if refs.len() != 1 {
        return Err(ApiError::bad_request(format!(
            "push must update exactly one branch and no tags; found {}",
            describe_refs(&refs)
        )));
    }
    let (branch, head_oid) = refs.into_iter().next().expect("length checked");
    ensure_default_branch(&branch)?;
    Ok((branch, head_oid))
}

pub(crate) const STALE_MAIN_PUSH_ERROR: &str =
    "main moved while this push was in flight; fetch, rebase, and push again";

fn ensure_pushed_head_extends(
    staging_repo: &FsPath,
    current_head: &str,
    pushed_head: &str,
) -> Result<(), ApiError> {
    let action = "checking that the push extends the current main head";
    let ancestry = run_git_output(
        Some(staging_repo),
        &["merge-base", "--is-ancestor", current_head, pushed_head],
        action,
    )?;
    match ancestry.status.code() {
        Some(0) => return Ok(()),
        Some(1) => return Err(ApiError::conflict(STALE_MAIN_PUSH_ERROR)),
        _ => {}
    }
    let current_commit = format!("{current_head}^{{commit}}");
    let present = run_git_output(
        Some(staging_repo),
        &["cat-file", "-e", &current_commit],
        "checking the current main head in the pushed objects",
    )?
    .status
    .success();
    if present {
        Err(ApiError::infrastructure_unavailable(format!(
            "{action}: {}",
            String::from_utf8_lossy(&ancestry.stderr).trim()
        )))
    } else {
        Err(ApiError::conflict(STALE_MAIN_PUSH_ERROR))
    }
}

struct InspectedReceiveTree {
    message: String,
    occurred_at_unix: Option<i64>,
    changes: Vec<ReceivePackFileChange>,
    landing_file_mutation: RepositoryLandingFileMutation,
    workflow_catalog: RepositoryWorkflowCatalog,
    diff_ms: u128,
}

fn inspect_received_tree(
    staging_repo: &FsPath,
    head_oid: &str,
    base_head_oid: Option<&str>,
    repository_id: &str,
    change_version: u64,
    mode: ReviewedUpdateMode,
) -> Result<InspectedReceiveTree, ApiError> {
    let PushedCommit {
        message,
        committed_at_unix,
    } = pushed_commit(staging_repo, head_oid)?;
    let occurred_at_unix = Some(committed_at_unix);
    validate_pushed_commit_range(staging_repo, base_head_oid, head_oid)?;
    let diff_started = Instant::now();
    let pushed_entries = git_changed_tree_entries(staging_repo, base_head_oid, head_oid)?;
    let diff_ms = diff_started.elapsed().as_millis();
    if pushed_entries.is_empty() && mode != ReviewedUpdateMode::RequestMerge {
        return Err(ApiError::bad_request(
            "receive-pack update did not change the live tree",
        ));
    }
    let landing_file_mutation = repository_landing_file_mutation(staging_repo, &pushed_entries)?;
    let workflow_catalog =
        capture_repository_workflow_catalog(staging_repo, repository_id, head_oid, change_version)?;
    let changes = pushed_entries
        .into_iter()
        .map(|(path, entry)| ReceivePackFileChange {
            path,
            content: entry.map(|entry| git_blob_reference(entry.oid, entry.mode, entry.size_bytes)),
        })
        .collect();
    Ok(InspectedReceiveTree {
        message,
        occurred_at_unix,
        changes,
        landing_file_mutation,
        workflow_catalog,
        diff_ms,
    })
}

fn repository_landing_file_mutation(
    staging_repo: &FsPath,
    pushed_entries: &[(ScopePath, Option<GitTreeFile>)],
) -> Result<RepositoryLandingFileMutation, ApiError> {
    let Some((_, entry)) = pushed_entries
        .iter()
        .find(|(path, _)| path.as_str() == REPOSITORY_LANDING_FILE_PATH)
    else {
        return Ok(RepositoryLandingFileMutation::Unchanged);
    };
    let Some(entry) = entry else {
        return Ok(RepositoryLandingFileMutation::Delete);
    };
    if entry.size_bytes > MAX_REPOSITORY_LANDING_FILE_BYTES as u64 {
        return Ok(RepositoryLandingFileMutation::Delete);
    }

    let source = git_blob_reference(entry.oid.clone(), entry.mode.clone(), entry.size_bytes);
    let output = run_git_output_bounded(
        Some(staging_repo),
        &["cat-file", "blob", &entry.oid],
        "reading repository landing file",
        MAX_REPOSITORY_LANDING_FILE_BYTES,
    )?;
    if !output.status.success() || output.stdout.len() as u64 != entry.size_bytes {
        return Err(ApiError::infrastructure_unavailable(
            "reading repository landing file failed",
        ));
    }
    RepositoryLandingFile::from_source_blob(&source, output.stdout)
        .map(RepositoryLandingFileMutation::Upsert)
        .map_err(ApiError::from)
}

pub(crate) enum ReadWorkflowFiles {
    Files(Vec<RepositoryWorkflowFile>),
    Rejected(String),
}

pub(crate) fn read_repository_workflow_files(
    staging_repo: &FsPath,
    head_oid: &str,
) -> Result<ReadWorkflowFiles, ApiError> {
    let workflow_entries = git_tree_entries_under(staging_repo, head_oid, ".scope/runs")?;
    if workflow_entries.len() > MAX_REPOSITORY_WORKFLOW_FILES {
        return Ok(ReadWorkflowFiles::Rejected(format!(
            "repository contains more than {MAX_REPOSITORY_WORKFLOW_FILES} workflow definitions"
        )));
    }

    for entry in &workflow_entries {
        let path = format!("/{}", entry.path);
        if WorkflowPath::parse(path.clone()).is_err() {
            return Ok(ReadWorkflowFiles::Rejected(format!(
                "invalid workflow path {path}"
            )));
        }
        if entry.size_bytes > MAX_WORKFLOW_DEFINITION_BYTES as u64 {
            return Ok(ReadWorkflowFiles::Rejected(format!(
                "workflow {path} exceeds {MAX_WORKFLOW_DEFINITION_BYTES} bytes"
            )));
        }
    }
    if workflow_entries.is_empty() {
        return Ok(ReadWorkflowFiles::Files(Vec::new()));
    }
    let input = workflow_entries
        .iter()
        .map(|entry| format!("{}\n", entry.oid))
        .collect::<String>()
        .into_bytes();
    let max_stdout_bytes = workflow_entries
        .iter()
        .map(|entry| entry.size_bytes as usize + entry.oid.len() + 32)
        .sum();
    let output = git_process_output(
        Command::new("git")
            .arg("-C")
            .arg(staging_repo)
            .args(["cat-file", "--batch"]),
        Some(input),
        ProcessLimits::new(RuntimeBudgets::default_git_command_timeout())
            .with_max_stdout_bytes(max_stdout_bytes),
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(
            "reading repository workflow definitions failed",
        ));
    }
    let mut remaining = output.stdout.as_slice();
    let mut workflows = Vec::with_capacity(workflow_entries.len());
    for entry in workflow_entries {
        let path = format!("/{}", entry.path);
        let header_end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| {
                ApiError::infrastructure_unavailable("invalid repository workflow batch response")
            })?;
        let expected_header = format!("{} blob {}", entry.oid, entry.size_bytes);
        if &remaining[..header_end] != expected_header.as_bytes() {
            return Err(ApiError::infrastructure_unavailable(
                "repository workflow batch response does not match tree metadata",
            ));
        }
        remaining = &remaining[header_end + 1..];
        let size = entry.size_bytes as usize;
        if remaining.get(size) != Some(&b'\n') {
            return Err(ApiError::infrastructure_unavailable(
                "truncated repository workflow batch response",
            ));
        }
        let source = git_blob_reference(entry.oid, entry.mode, entry.size_bytes);
        workflows.push(
            RepositoryWorkflowFile::from_source_blob(path, &source, remaining[..size].to_vec())
                .map_err(ApiError::internal)?,
        );
        remaining = &remaining[size + 1..];
    }
    if !remaining.is_empty() {
        return Err(ApiError::infrastructure_unavailable(
            "unexpected trailing repository workflow batch response",
        ));
    }
    Ok(ReadWorkflowFiles::Files(workflows))
}

fn capture_repository_workflow_catalog(
    staging_repo: &FsPath,
    repository_id: &str,
    head_oid: &str,
    change_version: u64,
) -> Result<RepositoryWorkflowCatalog, ApiError> {
    match read_repository_workflow_files(staging_repo, head_oid)? {
        ReadWorkflowFiles::Files(workflows) => {
            RepositoryWorkflowCatalog::captured(repository_id, head_oid, change_version, workflows)
        }
        ReadWorkflowFiles::Rejected(message) => {
            RepositoryWorkflowCatalog::rejected(repository_id, head_oid, change_version, message)
        }
    }
    .map_err(ApiError::internal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::{
        command::{run_git, run_git_output},
        import::validate_pushed_file_path,
        test_repo::TestRepo,
    };
    use scope_domain::{content::DEFAULT_GIT_FILE_MODE, policy::ScopePath};
    use std::fs;

    #[test]
    fn stale_main_push_is_a_conflict_before_any_pack_work() {
        let repo = TestRepo::init();
        fs::write(repo.join("a.txt"), b"a").unwrap();
        repo.commit_all("base");
        let base = repo.oid("HEAD");
        fs::write(repo.join("b.txt"), b"b").unwrap();
        repo.commit_all("pushed");
        let pushed = repo.oid("HEAD");
        ensure_pushed_head_extends(&repo, &base, &pushed).unwrap();

        run_git(
            Some(&repo),
            &["checkout", "-q", "-b", "other", &base],
            "branching",
        )
        .unwrap();
        fs::write(repo.join("c.txt"), b"c").unwrap();
        repo.commit_all("concurrent main");
        let concurrent = repo.oid("HEAD");
        for current in [
            concurrent.as_str(),
            "1111111111111111111111111111111111111111",
        ] {
            let error = ensure_pushed_head_extends(&repo, current, &pushed).unwrap_err();
            assert_eq!(error.status(), axum::http::StatusCode::CONFLICT);
            assert_eq!(error.into_public_message(), STALE_MAIN_PUSH_ERROR);
        }
    }

    #[test]
    fn landing_file_mutation_distinguishes_unchanged_delete_and_oversized() {
        assert_eq!(
            repository_landing_file_mutation(FsPath::new("unused"), &[]).unwrap(),
            RepositoryLandingFileMutation::Unchanged
        );

        let path = ScopePath::parse(REPOSITORY_LANDING_FILE_PATH).unwrap();
        assert_eq!(
            repository_landing_file_mutation(FsPath::new("unused"), &[(path.clone(), None)],)
                .unwrap(),
            RepositoryLandingFileMutation::Delete
        );

        let oversized = GitTreeFile {
            path: validate_pushed_file_path("README.html").unwrap(),
            mode: DEFAULT_GIT_FILE_MODE.to_string(),
            oid: "unused".to_string(),
            size_bytes: MAX_REPOSITORY_LANDING_FILE_BYTES as u64 + 1,
        };
        assert_eq!(
            repository_landing_file_mutation(FsPath::new("unused"), &[(path, Some(oversized))],)
                .unwrap(),
            RepositoryLandingFileMutation::Delete
        );
    }

    #[test]
    fn landing_file_upsert_reads_the_changed_git_blob() {
        let repo = TestRepo::init();
        let bytes = b"<!doctype html><h1>fast</h1>";
        fs::write(repo.join("README.html"), bytes).unwrap();
        let output = run_git_output(
            Some(&repo),
            &["hash-object", "-w", "README.html"],
            "writing landing file test blob",
        )
        .unwrap();
        assert!(output.status.success());
        let oid = String::from_utf8(output.stdout).unwrap().trim().to_string();
        let entry = GitTreeFile {
            path: validate_pushed_file_path("README.html").unwrap(),
            mode: DEFAULT_GIT_FILE_MODE.to_string(),
            oid: oid.clone(),
            size_bytes: bytes.len() as u64,
        };

        let mutation = repository_landing_file_mutation(
            &repo,
            &[(
                ScopePath::parse(REPOSITORY_LANDING_FILE_PATH).unwrap(),
                Some(entry),
            )],
        )
        .unwrap();
        let RepositoryLandingFileMutation::Upsert(file) = mutation else {
            panic!("expected landing file upsert");
        };
        assert_eq!(file.oid, oid);
        assert_eq!(file.content_bytes, bytes);
        assert_eq!(file.size_bytes, bytes.len() as u64);
    }

    #[test]
    fn workflow_files_read_multiple_blobs_in_tree_order() {
        let repo = TestRepo::init();
        fs::create_dir_all(repo.join(".scope/runs")).unwrap();
        fs::write(repo.join(".scope/runs/first.yml"), b"first\n\0body").unwrap();
        fs::write(repo.join(".scope/runs/second.yaml"), b"second\nbody").unwrap();
        run_git(Some(&repo), &["add", "."], "staging workflow files").unwrap();
        let tree = run_git_output(Some(&repo), &["write-tree"], "writing workflow tree").unwrap();
        let tree_oid = String::from_utf8(tree.stdout).unwrap();
        let ReadWorkflowFiles::Files(files) =
            read_repository_workflow_files(&repo, tree_oid.trim()).unwrap()
        else {
            panic!("expected workflow files");
        };
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path().as_str(), "/.scope/runs/first.yml");
        assert_eq!(files[0].content_bytes(), b"first\n\0body");
        assert_eq!(files[1].path().as_str(), "/.scope/runs/second.yaml");
        assert_eq!(files[1].content_bytes(), b"second\nbody");

        fs::write(
            repo.join(".scope/runs/oversized.yml"),
            vec![b'x'; MAX_WORKFLOW_DEFINITION_BYTES + 1],
        )
        .unwrap();
        run_git(Some(&repo), &["add", "."], "staging oversized workflow").unwrap();
        let tree = run_git_output(Some(&repo), &["write-tree"], "writing oversized tree").unwrap();
        let tree_oid = String::from_utf8(tree.stdout).unwrap();
        assert!(matches!(
            read_repository_workflow_files(&repo, tree_oid.trim()).unwrap(),
            ReadWorkflowFiles::Rejected(reason) if reason.contains("exceeds")
        ));
    }
}
