use super::*;
use crate::git::{
    command::{git_is_ancestor, git_ref_listing},
    pool_install::present_commits,
    test_repo::pinned_identity_git,
};
use axum::http::StatusCode;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};
use tempfile::TempDir;

const MISSING_OID: &str = "1111111111111111111111111111111111111111";
const ZERO_OID: &str = "0000000000000000000000000000000000000000";

struct Fixture {
    _dir: TempDir,
    origin: PathBuf,
    stage: PathBuf,
    oids: BTreeMap<&'static str, String>,
}

impl Fixture {
    fn build() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let origin = dir.path().join("origin");
        git_ok(
            dir.path(),
            &["init", "-q", "-b", "main", origin.to_str().unwrap()],
        );
        let mut oids = BTreeMap::new();
        fs::write(origin.join("a.txt"), "a").unwrap();
        g(&origin, &["add", "."]);
        oids.insert("base", commit(&origin, "base"));
        oids.insert("c2", commit(&origin, "c2"));
        oids.insert("c3", commit(&origin, "c3"));
        g(&origin, &["checkout", "-q", "-b", "side", &oids["base"]]);
        oids.insert("s1", commit(&origin, "s1"));
        g(&origin, &["checkout", "-q", "-b", "x", &oids["base"]]);
        oids.insert("x1", commit(&origin, "x1"));
        g(&origin, &["checkout", "-q", "-b", "y", &oids["base"]]);
        oids.insert("y1", commit(&origin, "y1"));
        g(&origin, &["checkout", "-q", "x"]);
        g(&origin, &["merge", "-q", "--no-ff", "-m", "x2", "y"]);
        oids.insert("x2", g(&origin, &["rev-parse", "HEAD"]));
        g(&origin, &["checkout", "-q", "y"]);
        g(
            &origin,
            &["merge", "-q", "--no-ff", "-m", "y2", &oids["x1"]],
        );
        oids.insert("y2", g(&origin, &["rev-parse", "HEAD"]));
        g(&origin, &["checkout", "-q", "main"]);
        g(&origin, &["merge", "-q", "--no-ff", "-m", "m", "side"]);
        oids.insert("m", g(&origin, &["rev-parse", "HEAD"]));
        g(&origin, &["checkout", "-q", "--orphan", "lonely"]);
        oids.insert("lonely", commit(&origin, "lonely"));
        g(&origin, &["checkout", "-q", "main"]);
        g(&origin, &["tag", "-a", "v1", "-m", "v1", &oids["c2"]]);
        g(&origin, &["tag", "-a", "v1-again", "-m", "again", "v1"]);
        let base_tree = format!("{}^{{tree}}", oids["base"]);
        g(
            &origin,
            &["tag", "-a", "tree-tag", "-m", "tree", &base_tree],
        );
        oids.insert("tag", g(&origin, &["rev-parse", "v1"]));
        oids.insert("tree", g(&origin, &["rev-parse", &base_tree]));
        let blob = format!("{}:a.txt", oids["base"]);
        oids.insert("blob", g(&origin, &["rev-parse", &blob]));
        g(&origin, &["pack-refs", "--all"]);
        g(&origin, &["update-ref", "refs/heads/side", &oids["c3"]]);
        g(&origin, &["update-ref", "refs/heads/fresh", &oids["c2"]]);
        g(
            &origin,
            &["symbolic-ref", "refs/heads/alias", "refs/heads/main"],
        );
        fs::write(
            origin.join(".git/refs/heads/ghost"),
            format!("{MISSING_OID}\n"),
        )
        .unwrap();

        let mid = dir.path().join("mid");
        let stage = dir.path().join("stage");
        for (repo, alternate) in [
            (&mid, origin.join(".git/objects").display().to_string()),
            (&stage, "../../mid/objects".to_string()),
        ] {
            git_ok(
                dir.path(),
                &["init", "-q", "--bare", repo.to_str().unwrap()],
            );
            fs::write(
                repo.join("objects/info/alternates"),
                format!("{alternate}\n"),
            )
            .unwrap();
        }
        for (repo, name, parent) in [(&mid, "mid", "m"), (&stage, "staged", "mid")] {
            let tree = format!("{}^{{tree}}", oids["base"]);
            let oid = g(
                repo,
                &["commit-tree", &tree, "-p", &oids[parent], "-m", name],
            );
            g(repo, &["update-ref", "refs/heads/main", &oid]);
            oids.insert(name, oid);
        }
        Self {
            _dir: dir,
            origin,
            stage,
            oids,
        }
    }

    fn repos(&self) -> [&Path; 2] {
        [&self.origin, &self.stage]
    }

    fn oid(&self, name: &str) -> String {
        self.oids[name].clone()
    }
}

fn commit(repo: &Path, message: &str) -> String {
    g(repo, &["commit", "-q", "--allow-empty", "-m", message]);
    g(repo, &["rev-parse", "HEAD"])
}

fn g(repo: &Path, args: &[&str]) -> String {
    pinned_identity_git(repo, args, None)
}

fn git(repo: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .unwrap()
}

fn git_ok(repo: &Path, args: &[&str]) {
    assert!(git(repo, args).status.success(), "git {args:?}");
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn object_answers_match_cat_file_through_chained_relative_alternates() {
    let fixture = Fixture::build();
    let base = fixture.oid("base");
    let mut inputs = [
        "base", "c3", "s1", "x2", "m", "lonely", "mid", "staged", "tag", "tree", "blob",
    ]
    .map(|name| fixture.oid(name))
    .to_vec();
    inputs.extend([MISSING_OID, ZERO_OID].map(String::from));
    let unanswered = [
        base[..12].to_string(),
        base.to_uppercase(),
        "HEAD".to_string(),
    ];
    for repo in fixture.repos() {
        let reader = GitReader::open(repo).unwrap();
        let mut commits = BTreeSet::new();
        for oid in &inputs {
            let typed = git(repo, &["cat-file", "-t", oid]);
            let is_commit = typed.status.success() && stdout(&typed) == "commit";
            let peels = git(repo, &["cat-file", "-e", &format!("{oid}^{{commit}}")]);
            assert_eq!(reader.is_commit(oid), Some(is_commit), "{oid} in {repo:?}");
            assert_eq!(
                reader.peels_to_commit(oid),
                Some(peels.status.success()),
                "{oid}"
            );
            if is_commit {
                commits.insert(oid.clone());
            }
        }
        assert!(commits.contains(&base) && !commits.contains(&fixture.oid("tag")));
        let oids = inputs.iter().map(String::as_str).collect::<Vec<_>>();
        assert_eq!(reader.present_commits(&oids), Some(commits));
        for oid in &unanswered {
            assert_eq!(reader.is_commit(oid), None);
            assert_eq!(reader.peels_to_commit(oid), None);
        }
    }
    let all = inputs.iter().chain(&unanswered).map(String::as_str);
    let batch = git_batch_commits(&fixture.stage, all.clone());
    assert!(batch.contains(&base) && batch.contains(&fixture.oid("staged")));
    assert_eq!(present_commits(&fixture.stage, all).unwrap(), batch);
}

fn git_batch_commits<'a>(repo: &Path, oids: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let input = oids.map(|oid| format!("{oid}\n")).collect::<String>();
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "--batch-check=%(objectname) %(objecttype)"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write as _;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    stdout(&output)
        .lines()
        .filter_map(|line| line.strip_suffix(" commit"))
        .map(str::to_string)
        .collect()
}

#[test]
fn references_match_for_each_ref_and_rev_parse_across_packed_loose_and_symbolic_refs() {
    let fixture = Fixture::build();
    let names = [
        "refs/heads/main",
        "refs/heads/side",
        "refs/heads/fresh",
        "refs/heads/alias",
        "refs/heads/ghost",
        "refs/heads/absent",
        "refs/tags/v1",
        "refs/tags/v1-again",
        "refs/tags/tree-tag",
    ];
    let prefix_sets: [&[&str]; 5] = [
        &[],
        &["refs/heads"],
        &["refs/heads/main", "refs/tags"],
        &["refs/head"],
        &["refs/tags/"],
    ];
    for repo in fixture.repos() {
        let reader = GitReader::open(repo).unwrap();
        for prefixes in prefix_sets {
            assert_eq!(
                reader.references(prefixes),
                Some(git_for_each_ref(repo, prefixes)),
                "{prefixes:?} in {repo:?}"
            );
        }
        for name in names {
            let target = git(repo, &["rev-parse", "--verify", "--quiet", name]);
            let expected = target.status.success().then(|| stdout(&target));
            assert_eq!(reader.reference_target(name), Some(expected), "{name}");
            let peeled = format!("{name}^{{commit}}");
            let commit = git(repo, &["rev-parse", "--verify", "--quiet", &peeled]);
            let expected = commit.status.success().then(|| stdout(&commit));
            assert_eq!(reader.reference_commit(name), expected, "{name}");
        }
    }
    assert_eq!(
        GitReader::open(&fixture.origin)
            .unwrap()
            .reference_commit("refs/tags/v1-again"),
        Some(fixture.oid("c2"))
    );
    g(
        &fixture.origin,
        &["symbolic-ref", "refs/heads/dangling", "refs/heads/nowhere"],
    );
    assert_eq!(
        git_ref_listing(&fixture.origin, &["refs/heads"], "listing").unwrap(),
        git_for_each_ref(&fixture.origin, &["refs/heads"])
    );
}

fn git_for_each_ref(repo: &Path, prefixes: &[&str]) -> Vec<(String, String)> {
    let mut args = vec!["for-each-ref", "--format=%(refname) %(objectname)"];
    args.extend(prefixes);
    let output = git(repo, &args);
    assert!(output.status.success());
    stdout(&output)
        .lines()
        .map(|line| {
            let (name, oid) = line.split_once(' ').unwrap();
            (name.to_string(), oid.to_string())
        })
        .collect()
}

#[test]
fn ancestry_and_merge_bases_match_git_including_criss_cross_and_tags() {
    let fixture = Fixture::build();
    let mut revisions = [
        "base", "c3", "s1", "x2", "y2", "m", "lonely", "tag", "staged", "tree",
    ]
    .map(|name| fixture.oid(name))
    .to_vec();
    revisions.extend(["refs/heads/main", "refs/tags/v1", MISSING_OID].map(String::from));
    let mut several = 0;
    for repo in fixture.repos() {
        let reader = GitReader::open(repo).unwrap();
        for left in &revisions {
            for right in &revisions {
                let ancestry = git(repo, &["merge-base", "--is-ancestor", left, right]);
                let expected = match ancestry.status.code() {
                    Some(0) => Some(true),
                    Some(1) => Some(false),
                    _ => None,
                };
                assert_eq!(
                    reader.is_ancestor(left, right, "checking").unwrap(),
                    expected,
                    "{left} {right} in {repo:?}"
                );
                let all = git(repo, &["merge-base", "--all", left, right]);
                let bases = stdout(&all);
                let lines = bases.lines().collect::<Vec<_>>();
                let expected = match (all.status.code(), lines.as_slice()) {
                    (Some(1), []) => Some(MergeBases::Unrelated),
                    (Some(0), [base]) => Some(MergeBases::Unique(base.to_string())),
                    (Some(0), [_, _, ..]) => Some(MergeBases::Several),
                    _ => None,
                };
                several += usize::from(expected == Some(MergeBases::Several));
                assert_eq!(
                    reader.merge_bases(left, right, "reading").unwrap(),
                    expected,
                    "{left} {right} in {repo:?}"
                );
            }
        }
    }
    assert!(several > 0);
    let short = &fixture.oid("base")[..12];
    let reader = GitReader::open(&fixture.origin).unwrap();
    assert_eq!(
        reader
            .is_ancestor(short, "refs/heads/main", "checking")
            .unwrap(),
        None
    );
}

#[test]
fn walks_fail_like_a_timed_out_git_command_once_the_deadline_passes() {
    let fixture = Fixture::build();
    let reader = GitReader::open_with_budget(&fixture.stage, Duration::ZERO).unwrap();
    let base = fixture.oid("base");
    let staged = fixture.oid("staged");
    for error in [
        reader
            .is_ancestor(&base, &staged, "checking ancestry")
            .unwrap_err(),
        reader
            .merge_bases(&base, &staged, "checking ancestry")
            .unwrap_err(),
    ] {
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            error.operator_diagnostic(),
            "failed checking ancestry: Git reader timed out after 0ms"
        );
    }
}

#[test]
fn walks_that_reach_a_missing_parent_leave_the_answer_to_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git_ok(repo, &["init", "-q", "-b", "main"]);
    let first = commit(repo, "first");
    commit(repo, "second");
    let tip = commit(repo, "third");
    g(repo, &["checkout", "-q", "--orphan", "lonely"]);
    let lonely = commit(repo, "lonely");
    fs::remove_file(
        repo.join(".git/objects")
            .join(&first[..2])
            .join(&first[2..]),
    )
    .unwrap();

    let reader = GitReader::open(repo).unwrap();
    assert_eq!(reader.is_ancestor(&lonely, &tip, "checking").unwrap(), None);
    assert_eq!(reader.merge_bases(&lonely, &tip, "checking").unwrap(), None);
    let ancestry = git(repo, &["merge-base", "--is-ancestor", &lonely, &tip]);
    assert_eq!(ancestry.status.code(), Some(128));
    let error = git_is_ancestor(repo, &lonely, &tip, "checking").unwrap_err();
    assert!(error.operator_diagnostic().starts_with("checking: "));
}

#[test]
fn shallow_repositories_leave_ancestry_to_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git_ok(repo, &["init", "-q", "-b", "main"]);
    let first = commit(repo, "first");
    let boundary = commit(repo, "boundary");
    let tip = commit(repo, "tip");
    fs::write(repo.join(".git/shallow"), format!("{boundary}\n")).unwrap();

    assert!(GitReader::open(repo).is_none());
    let git_answer = git(repo, &["merge-base", "--is-ancestor", &first, &tip])
        .status
        .success();
    assert!(!git_answer);
    assert_eq!(
        git_is_ancestor(repo, &first, &tip, "checking").unwrap(),
        git_answer
    );
}

#[test]
fn tag_peeling_stops_on_a_replacement_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git_ok(repo, &["init", "-q", "-b", "main"]);
    commit(repo, "first");
    g(repo, &["tag", "-a", "-m", "outer", "outer"]);
    let outer = g(repo, &["rev-parse", "refs/tags/outer"]);
    g(repo, &["tag", "-a", "-m", "inner", "inner", &outer]);
    let inner = g(repo, &["rev-parse", "refs/tags/inner"]);
    g(repo, &["replace", "-f", &outer, &inner]);

    assert!(GitReader::open(repo).is_none());
    let reader = GitReader::open_with_budget_for_tests(repo);
    assert_eq!(reader.peels_to_commit(&outer), None);
    assert_eq!(reader.revision_commit(&outer), None);
}

#[test]
fn replacement_chains_leave_answers_to_git() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();
    git_ok(repo, &["init", "-q", "-b", "main"]);
    let first = commit(repo, "first");
    let tree = g(repo, &["rev-parse", "HEAD^{tree}"]);
    g(repo, &["tag", "-a", "-m", "a", "a"]);
    let a = g(repo, &["rev-parse", "refs/tags/a"]);
    g(repo, &["tag", "-a", "-m", "b", "b", &tree]);
    let b = g(repo, &["rev-parse", "refs/tags/b"]);
    g(repo, &["tag", "-a", "-m", "c", "c", &first]);
    let c = g(repo, &["rev-parse", "refs/tags/c"]);
    g(repo, &["replace", "-f", &a, &b]);
    g(repo, &["replace", "-f", &b, &c]);

    assert!(GitReader::open(repo).is_none());
    let git_answer = git(repo, &["cat-file", "-e", &format!("{a}^{{commit}}")])
        .status
        .success();
    assert_eq!(
        crate::git::command::git_contains_commit(repo, &a, &first, "checking").unwrap(),
        git_answer
            && git(repo, &["merge-base", "--is-ancestor", &a, &first])
                .status
                .success()
    );
}
