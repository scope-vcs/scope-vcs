use super::*;
use crate::{git::test_repo::pinned_identity_git, runtime_budgets::RuntimeBudgets};
use scope_domain::repository::git::GitPoolPackOwner;
use std::{fs, path::Path};

fn git(repo: &Path, args: &[&str]) -> String {
    pinned_identity_git(repo, args, None)
}

fn pack(repo: &Path, head: &str, range: &str) -> (GitPoolPack, FetchedPack) {
    let bytes = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["pack-objects", "--stdout", "--revs"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(format!("{range}\n").as_bytes())?;
            child.wait_with_output()
        })
        .unwrap()
        .stdout;
    (
        GitPoolPack {
            id: head.to_string(),
            owner: GitPoolPackOwner::Main {
                first_sequence: 1,
                last_sequence: 1,
            },
            head_oid: head.to_string(),
            prerequisites: Vec::new(),
            storage: GitPoolPackStorage::Inline { sealed: Vec::new() },
        },
        FetchedPack::Bytes(bytes),
    )
}

fn pack_files(shelf: &Path) -> usize {
    fs::read_dir(shelf.join("objects/pack"))
        .unwrap()
        .filter(|entry| {
            entry
                .as_ref()
                .unwrap()
                .path()
                .extension()
                .is_some_and(|extension| extension == "pack")
        })
        .count()
}

#[test]
fn a_batch_indexes_only_packs_whose_heads_are_still_missing() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let shelf = dir.path().join("shelf.git");
    git(
        dir.path(),
        &["init", "-q", "-b", "main", source.to_str().unwrap()],
    );
    git(
        dir.path(),
        &["init", "-q", "--bare", shelf.to_str().unwrap()],
    );
    fs::write(source.join("a.txt"), "a").unwrap();
    git(&source, &["add", "a.txt"]);
    git(&source, &["commit", "-q", "-m", "a"]);
    let first = git(&source, &["rev-parse", "HEAD"]);
    fs::write(source.join("b.txt"), "b").unwrap();
    git(&source, &["add", "b.txt"]);
    git(&source, &["commit", "-q", "-m", "b"]);
    let second = git(&source, &["rev-parse", "HEAD"]);
    let timeout = RuntimeBudgets::default_git_command_timeout();

    let installed = index_missing(&shelf, vec![pack(&source, &first, &first)], timeout).unwrap();
    assert_eq!(installed[0].1.as_ref().unwrap(), &PackInstall::Indexed);
    assert_eq!(pack_files(&shelf), 1);

    let batch = vec![
        pack(&source, &first, &first),
        pack(&source, &second, &format!("{second}\n^{first}")),
    ];
    let outcomes = index_missing(&shelf, batch, timeout)
        .unwrap()
        .into_iter()
        .map(|(pack, outcome)| (pack.head_oid, outcome.unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(
        outcomes,
        vec![
            (first, PackInstall::AlreadyPresent),
            (second.clone(), PackInstall::Indexed),
        ]
    );
    assert_eq!(pack_files(&shelf), 2);
    assert!(
        present_commits(&shelf, std::iter::once(second.as_str()))
            .unwrap()
            .contains(&second)
    );
}
