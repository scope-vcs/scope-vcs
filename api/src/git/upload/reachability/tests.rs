use super::*;
use crate::git::test_repo::pinned_identity_git;
use std::fs;

const MISSING: &[u8] = b"1111111111111111111111111111111111111111";

struct Folder {
    _dir: tempfile::TempDir,
    folder: std::path::PathBuf,
    store: std::path::PathBuf,
}

impl Folder {
    fn build() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let folder = dir.path().join("folder.git");
        git(
            dir.path(),
            &["init", "-q", "-b", "main", store.to_str().unwrap()],
        );
        git(
            dir.path(),
            &["init", "-q", "--bare", folder.to_str().unwrap()],
        );
        fs::write(
            folder.join("objects/info/alternates"),
            format!("{}\n", store.join(".git/objects").display()),
        )
        .unwrap();
        Self {
            _dir: dir,
            folder,
            store,
        }
    }

    fn commit(&self, file: &str, content: &str) -> Vec<u8> {
        fs::write(self.store.join(file), content).unwrap();
        git(&self.store, &["add", file]);
        git(&self.store, &["commit", "-q", "-m", file]);
        rev(&self.store, "HEAD")
    }

    fn advertise(&self, name: &str, oid: &[u8]) {
        git(
            &self.folder,
            &["update-ref", name, std::str::from_utf8(oid).unwrap()],
        );
    }

    fn oracle(&self, candidates: &BTreeSet<Vec<u8>>) -> BTreeSet<Vec<u8>> {
        let walked = git(&self.folder, &["rev-list", "--objects", "--all"]);
        let reachable = walked
            .lines()
            .map(|line| {
                line.split(' ')
                    .next()
                    .unwrap_or_default()
                    .as_bytes()
                    .to_vec()
            })
            .collect::<BTreeSet<_>>();
        candidates.difference(&reachable).cloned().collect()
    }
}

fn rev(repo: &std::path::Path, revision: &str) -> Vec<u8> {
    git(repo, &["rev-parse", revision]).into_bytes()
}

fn git(repo: &std::path::Path, args: &[&str]) -> String {
    pinned_identity_git(repo, args, None)
}

fn set(oids: &[&[u8]]) -> BTreeSet<Vec<u8>> {
    oids.iter().map(|oid| oid.to_vec()).collect()
}

#[test]
fn commit_candidates_match_the_full_walk_through_alternates() {
    let folder = Folder::build();
    let first = folder.commit("a.txt", "a");
    let second = folder.commit("b.txt", "b");
    let tip = folder.commit("c.txt", "c");
    git(&folder.store, &["checkout", "-q", "-b", "hidden", "HEAD~2"]);
    let hidden = folder.commit("secret.txt", "secret");
    let hidden_child = folder.commit("secret2.txt", "secret2");
    folder.advertise("refs/heads/main", &tip);

    let candidates = set(&[&first, &second, &tip, &hidden, &hidden_child, MISSING]);
    let unreachable = unreachable_from_refs(&folder.folder, &candidates).unwrap();
    assert_eq!(unreachable, folder.oracle(&candidates));
    assert_eq!(unreachable, set(&[&hidden, &hidden_child, MISSING]));
}

#[test]
fn tree_and_blob_candidates_match_the_full_walk_through_alternates() {
    let folder = Folder::build();
    let first = folder.commit("a.txt", "a");
    let tip = folder.commit("b.txt", "b");
    git(&folder.store, &["checkout", "-q", "-b", "hidden", "HEAD~1"]);
    folder.commit("secret.txt", "secret");
    folder.advertise("refs/heads/main", &tip);
    let deep_blob = rev(
        &folder.store,
        &format!("{}:a.txt", String::from_utf8_lossy(&first)),
    );
    let old_tree = rev(
        &folder.store,
        &format!("{}^{{tree}}", String::from_utf8_lossy(&first)),
    );
    let hidden_blob = rev(&folder.store, "hidden:secret.txt");
    let hidden_tree = rev(&folder.store, "hidden^{tree}");

    let candidates = set(&[
        &first,
        &deep_blob,
        &old_tree,
        &hidden_blob,
        &hidden_tree,
        MISSING,
    ]);
    let unreachable = unreachable_from_refs(&folder.folder, &candidates).unwrap();
    assert_eq!(unreachable, folder.oracle(&candidates));
    assert_eq!(unreachable, set(&[&hidden_blob, &hidden_tree, MISSING]));
}

#[test]
fn every_advertised_ref_counts_as_visible() {
    let folder = Folder::build();
    let first = folder.commit("a.txt", "a");
    let tip = folder.commit("b.txt", "b");
    git(
        &folder.store,
        &["checkout", "-q", "-b", "request", "HEAD~1"],
    );
    let request = folder.commit("r.txt", "r");
    let request_tip = folder.commit("r2.txt", "r2");
    folder.advertise("refs/heads/main", &tip);
    folder.advertise("refs/scope/fetch/retained", &request_tip);

    let candidates = set(&[&first, &request]);
    let unreachable = unreachable_from_refs(&folder.folder, &candidates).unwrap();
    assert_eq!(unreachable, folder.oracle(&candidates));
    assert!(unreachable.is_empty());
}
