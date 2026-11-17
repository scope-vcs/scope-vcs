use super::git_command_output;
use crate::error::ApiError;
use scope_domain::{
    content_ref::ContentRef,
    projection::{ProjectedCommit, ProjectionMaterialization},
};
use std::{collections::BTreeSet, path::Path, process::Command};

pub(super) fn copy_generated_git_blobs(
    source_repo: &Path,
    projection_repo: &Path,
    commits: &[ProjectedCommit],
) -> Result<(), ApiError> {
    let oids = commits
        .iter()
        .filter(|commit| matches!(commit.materialization, ProjectionMaterialization::Generate))
        .flat_map(|commit| &commit.changes)
        .filter_map(|change| change.new_content.as_ref())
        .filter(|blob| matches!(blob.content_ref, ContentRef::GitBlob { .. }))
        .map(|blob| blob.git_oid.as_str())
        .collect::<BTreeSet<_>>();
    if oids.is_empty() {
        return Ok(());
    }
    let mut listed = String::with_capacity(oids.len() * 41);
    for oid in oids {
        listed.push_str(oid);
        listed.push('\n');
    }
    git_command_output(
        Command::new("git")
            .arg("--git-dir")
            .arg(projection_repo)
            .env(
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                source_repo.join("objects"),
            )
            .args(["pack-objects", "--quiet"])
            .arg(projection_repo.join("objects/pack/pack")),
        Some(listed.as_bytes()),
    )?;
    Ok(())
}
