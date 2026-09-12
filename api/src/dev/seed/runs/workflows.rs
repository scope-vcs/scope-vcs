use super::*;

pub(super) fn checks_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let container = seed_container()?;
    let build = WorkflowJob::new(
        job_id("build")?,
        vec![],
        container.clone(),
        600,
        vec![seed_cache()?],
        BTreeMap::new(),
        vec![step("Build", "cargo build --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let test = WorkflowJob::new(
        job_id("test")?,
        vec![job_id("build")?],
        container.clone(),
        600,
        vec![],
        BTreeMap::new(),
        vec![step("Test", "cargo test --workspace")?],
    )
    .map_err(ApiError::internal)?;
    let deploy = WorkflowJob::new(
        job_id("deploy")?,
        vec![job_id("test")?],
        container,
        900,
        vec![],
        BTreeMap::new(),
        vec![
            step("Package", "scripts/package.sh")?,
            step("Push image", "scripts/push-image.sh")?,
            step("Roll out", "scripts/roll-out.sh")?,
        ],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Checks",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![build, test, deploy],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/checks.yml", definition)
}

pub(super) fn lint_workflow_revision(repo_id: &str) -> Result<WorkflowRevision, ApiError> {
    let lint = WorkflowJob::new(
        job_id("lint")?,
        vec![],
        seed_container()?,
        300,
        vec![],
        BTreeMap::new(),
        vec![step("Lint", "scripts/lint.sh")?],
    )
    .map_err(ApiError::internal)?;
    let definition = CompiledWorkflow::new(
        "Lint",
        WorkflowTriggers::new(true, true).map_err(ApiError::internal)?,
        vec![lint],
    )
    .map_err(ApiError::internal)?;
    workflow_revision(repo_id, "/.scope/runs/lint.yml", definition)
}

pub(super) fn workflow_revision(
    repo_id: &str,
    path: &str,
    definition: CompiledWorkflow,
) -> Result<WorkflowRevision, ApiError> {
    let identity = WorkflowIdentity::new(
        repo_id.to_string(),
        WorkflowPath::parse(path).map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)?;
    WorkflowRevision::new(identity, definition).map_err(ApiError::internal)
}

pub(super) fn seed_container() -> Result<ContainerSpec, ApiError> {
    ContainerSpec::new(format!(
        "ghcr.io/scope/dev-seed-ci@sha256:{}",
        fake_digest("scope-dev-seed-container-image")
    ))
    .map_err(ApiError::internal)
}

pub(super) fn seed_cache() -> Result<WorkflowCache, ApiError> {
    WorkflowCache::new(
        "cargo",
        "/root/.cache/cargo",
        "cargo-v1",
        CacheKeyInputs::new(vec!["Cargo.lock".to_string()], vec![], false)
            .map_err(ApiError::internal)?,
        CacheKeyInputs::new(vec!["Cargo.lock".to_string()], vec![], true)
            .map_err(ApiError::internal)?,
    )
    .map_err(ApiError::internal)
}

pub(super) fn job_id(id: &str) -> Result<WorkflowJobId, ApiError> {
    WorkflowJobId::parse(id).map_err(ApiError::internal)
}

pub(super) fn step(name: &str, run: &str) -> Result<WorkflowStep, ApiError> {
    WorkflowStep::new(name, run).map_err(ApiError::internal)
}
