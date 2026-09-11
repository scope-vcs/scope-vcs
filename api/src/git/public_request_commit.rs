use crate::git::import::require_git_success;
use std::{collections::BTreeMap, path::Path, time::Instant};

use scope_domain::{
    policy::Visibility,
    projection::{FileChange, NativePublicCommit, NativePublicCommitDetails},
};

use crate::{
    error::ApiError,
    git::{
        content::git_blob_reference,
        import::{git_changed_tree_entries, remaining_git_time, run_git_output_until},
    },
};

/// Read history from the original Git objects, checking the persisted identity
/// before using their metadata. Safety-validated changed_paths has a different
/// baseline from the first-parent diff used to display a merge commit.
pub(crate) fn inspect_native_public_commit(
    repo: &Path,
    native: &NativePublicCommit,
    deadline: Instant,
) -> Result<NativePublicCommitDetails, ApiError> {
    let output = run_git_output_until(
        Some(repo),
        &[
            "show",
            "-s",
            "--format=%H%x00%T%x00%P%x00%ct%x00%an <%ae>%x00%B",
            &native.oid,
        ],
        "reading public request commit metadata",
        deadline,
    )?;
    let output = require_git_success(output, "reading public request commit metadata")?;
    // Split only the metadata separators: display text can contain NULs and
    // non-UTF-8 bytes, while the retained Git identity must remain strict.
    let fields = output
        .stdout
        .splitn(6, |byte| *byte == 0)
        .collect::<Vec<_>>();
    if fields.len() != 6 {
        return Err(ApiError::internal_message(
            "invalid public request commit metadata",
        ));
    }
    let parents = std::str::from_utf8(fields[2]).map_err(ApiError::internal)?;
    if fields[0] != native.oid.as_bytes()
        || fields[1] != native.tree_oid.as_bytes()
        || parents.split_ascii_whitespace().collect::<Vec<_>>() != native.parent_oids
    {
        return Err(ApiError::internal_message(
            "public request commit does not match its recorded Git identity",
        ));
    }
    let occurred_at_unix = std::str::from_utf8(fields[3])
        .map_err(ApiError::internal)?
        .parse()
        .map_err(|_| ApiError::internal_message("invalid public request commit time"))?;
    let author = String::from_utf8_lossy(fields[4]).into_owned();
    let message = String::from_utf8_lossy(fields[5])
        .trim_end_matches(&['\r', '\n'][..])
        .to_string();
    let first_parent = native.parent_oids.first().map(String::as_str);
    let new_entries = git_changed_tree_entries(repo, first_parent, &native.oid, deadline)?;
    let mut old_entries = match first_parent {
        Some(parent) => git_changed_tree_entries(repo, Some(&native.oid), parent, deadline)?
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
    remaining_git_time(deadline)?;
    Ok(NativePublicCommitDetails {
        author,
        message,
        occurred_at_unix,
        changes,
    })
}

#[cfg(test)]
#[path = "public_request_commit_tests.rs"]
mod tests;
