use super::{HISTORY_GENERATION_VERSION, HistoryEntry, HistoryEntryKind};
use crate::{content::SourceBlob, views::ViewId};
use sha2::{Digest, Sha256};

pub(super) fn history_generation_start(repo_id: &str, view: &ViewId) -> String {
    let mut hasher = Sha256::new();
    hash_field(
        &mut hasher,
        b"semantics",
        HISTORY_GENERATION_VERSION.as_bytes(),
    );
    hash_field(&mut hasher, b"repo", repo_id.as_bytes());
    hash_field(&mut hasher, b"view", view.as_str().as_bytes());
    hex::encode(hasher.finalize())
}

pub(super) fn history_generation_after(generation: &str, entry: &HistoryEntry) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"generation", generation.as_bytes());
    hash_field(&mut hasher, b"entry", entry.id.as_bytes());
    hash_field(&mut hasher, b"source", entry.source_id.as_bytes());
    hash_field(
        &mut hasher,
        b"kind",
        match entry.kind {
            HistoryEntryKind::Push => b"push",
            HistoryEntryKind::MergedRequest => b"merged_request",
            HistoryEntryKind::VisibilityChange => b"visibility_change",
            HistoryEntryKind::ViewsChange => b"views_change",
        },
    );
    hash_optional_field(&mut hasher, b"parent", entry.parent_id.as_deref());
    hash_optional_field(&mut hasher, b"author", entry.author.as_deref());
    hash_optional_field(
        &mut hasher,
        b"occurred_at",
        entry
            .occurred_at_unix
            .map(|time| time.to_string())
            .as_deref(),
    );
    hash_field(&mut hasher, b"message", entry.message.as_bytes());
    for commit in &entry.native_commits {
        hash_field(&mut hasher, b"native_oid", commit.oid.as_bytes());
        hash_field(&mut hasher, b"native_tree", commit.tree_oid.as_bytes());
        for parent in &commit.parent_oids {
            hash_field(&mut hasher, b"native_parent", parent.as_bytes());
        }
    }
    for file in &entry.files {
        hash_field(&mut hasher, b"path", file.path.as_str().as_bytes());
        hash_field(&mut hasher, b"visibility", file.label.as_str().as_bytes());
        hash_optional_blob(&mut hasher, b"old", file.old_content.as_ref());
        hash_optional_blob(&mut hasher, b"new", file.new_content.as_ref());
    }
    for change in &entry.visibility_changes {
        hash_field(&mut hasher, b"visibility_change_id", change.id.as_bytes());
        if let Some(file) = &change.file {
            hash_optional_blob(&mut hasher, b"visibility_old", file.old_content.as_ref());
            hash_optional_blob(&mut hasher, b"visibility_new", file.new_content.as_ref());
        }
        hash_field(
            &mut hasher,
            b"visibility_path",
            change.path.as_str().as_bytes(),
        );
        hash_field(
            &mut hasher,
            b"old_label",
            change.old_label.as_str().as_bytes(),
        );
        hash_field(
            &mut hasher,
            b"new_label",
            change.new_label.as_str().as_bytes(),
        );
    }
    if let Some(transition) = &entry.views {
        for (label, views) in [
            (b"views_before".as_slice(), &transition.before),
            (b"views_after".as_slice(), &transition.after),
        ] {
            let json = serde_json::to_vec(views).expect("views serialize to JSON");
            hash_field(&mut hasher, label, &json);
        }
    }
    hex::encode(hasher.finalize())
}

fn hash_optional_blob(hasher: &mut Sha256, label: &[u8], blob: Option<&SourceBlob>) {
    match blob {
        Some(blob) => {
            hash_field(hasher, label, b"present");
            hash_field(hasher, b"sha256", blob.sha256.as_bytes());
            hash_field(hasher, b"git_oid", blob.git_oid.as_bytes());
            hash_field(hasher, b"mode", blob.git_file_mode.as_bytes());
            hash_field(hasher, b"size", blob.size_bytes.to_string().as_bytes());
        }
        None => hash_field(hasher, label, b"absent"),
    }
}

fn hash_optional_field(hasher: &mut Sha256, label: &[u8], value: Option<&str>) {
    match value {
        Some(value) => {
            hash_field(hasher, label, b"present");
            hash_field(hasher, label, value.as_bytes());
        }
        None => hash_field(hasher, label, b"absent"),
    }
}

fn hash_field(hasher: &mut Sha256, label: &[u8], value: &[u8]) {
    hasher.update((label.len() as u64).to_be_bytes());
    hasher.update(label);
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}
