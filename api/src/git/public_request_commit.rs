use std::{collections::BTreeMap, path::Path};

use scope_domain::{
    policy::Visibility,
    projection::{FileChange, NativePublicCommit, NativePublicCommitDetails},
};

use crate::{
    error::ApiError,
    git::{
        content::git_blob_reference,
        import::{git_changed_tree_entries, git_stdout_text, run_git_output},
    },
};

/// Read history from the original Git objects, checking the persisted identity
/// before using their metadata. Safety-validated changed_paths has a different
/// baseline from the first-parent diff used to display a merge commit.
pub(crate) fn inspect_native_public_commit(
    repo: &Path,
    native: &NativePublicCommit,
) -> Result<NativePublicCommitDetails, ApiError> {
    let oid = git_stdout_text(
        repo,
        &[
            "rev-parse",
            "--verify",
            &format!("{}^{{commit}}", native.oid),
        ],
        "resolving public request commit",
    )?;
    let tree_oid = commit_field(repo, &native.oid, "%T")?;
    let parents = commit_field(repo, &native.oid, "%P")?;
    let parent_oids = parents.split_ascii_whitespace().collect::<Vec<_>>();
    if oid.trim() != native.oid
        || tree_oid.trim() != native.tree_oid
        || parent_oids != native.parent_oids
    {
        return Err(ApiError::internal_message(
            "public request commit does not match its recorded Git identity",
        ));
    }
    let author = display_commit_field(repo, &native.oid, "%an <%ae>")?;
    let message = display_commit_field(repo, &native.oid, "%B")?;
    let occurred_at_unix = commit_field(repo, &native.oid, "%ct")?
        .trim()
        .parse()
        .map_err(|_| ApiError::internal_message("invalid public request commit time"))?;
    let first_parent = native.parent_oids.first().map(String::as_str);
    let new_entries = git_changed_tree_entries(repo, first_parent, &native.oid)?;
    let mut old_entries = match first_parent {
        Some(parent) => git_changed_tree_entries(repo, Some(&native.oid), parent)?
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        None => BTreeMap::new(),
    };
    let changes = new_entries
        .into_iter()
        .map(|(path, new_entry)| {
            let old_entry = old_entries.remove(&path).flatten();
            FileChange {
                path,
                old_content: old_entry
                    .map(|entry| git_blob_reference(entry.oid, entry.mode, entry.size_bytes)),
                new_content: new_entry
                    .map(|entry| git_blob_reference(entry.oid, entry.mode, entry.size_bytes)),
                visibility: Visibility::Public,
            }
        })
        .collect();
    Ok(NativePublicCommitDetails {
        author,
        message,
        occurred_at_unix,
        changes,
    })
}

fn commit_field(repo: &Path, oid: &str, format: &str) -> Result<String, ApiError> {
    git_stdout_text(
        repo,
        &["show", "-s", &format!("--format={format}"), oid],
        "reading public request commit metadata",
    )
    .map(|value| value.trim_end_matches(&['\r', '\n'][..]).to_string())
}

// Git permits non-UTF-8 display text. Match request review's lossy decoding;
// identity fields above remain strict and are checked against retained refs.
fn display_commit_field(repo: &Path, oid: &str, format: &str) -> Result<String, ApiError> {
    let output = run_git_output(
        Some(repo),
        &["show", "-s", &format!("--format={format}"), oid],
        "reading public request commit display metadata",
    )?;
    if !output.status.success() {
        return Err(ApiError::infrastructure_unavailable(format!(
            "reading public request commit display metadata: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches(&['\r', '\n'][..])
        .to_string())
}

#[cfg(test)]
#[path = "public_request_commit_tests.rs"]
mod tests;
