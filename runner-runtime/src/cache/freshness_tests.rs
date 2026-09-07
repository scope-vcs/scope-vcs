use super::{
    archive::{create_archive, extract_archive},
    files::set_modified,
    sources::SourceSnapshot,
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime},
};

struct Fixture {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    target: PathBuf,
    bundle: PathBuf,
    archive: PathBuf,
    source: String,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let seed = temp.path().join("seed");
        let workspace = temp.path().join("workspace");
        let target = temp.path().join("target");
        let bundle = temp.path().join("source.bundle");
        let archive = temp.path().join("cache.tar.zst");
        fs::create_dir_all(seed.join("src")).unwrap();
        fs::create_dir_all(seed.join("dep/src")).unwrap();
        fs::write(seed.join("Cargo.toml"), "[package]\nname = 'cache-probe'\nversion = '0.1.0'\nedition = '2024'\n[dependencies]\nprobe-dep = { path = 'dep' }\n").unwrap();
        fs::write(
            seed.join("dep/Cargo.toml"),
            "[package]\nname = 'probe-dep'\nversion = '0.1.0'\nedition = '2024'\n",
        )
        .unwrap();
        fs::write(seed.join("dep/src/lib.rs"), "pub fn value() -> u8 { 1 }\n").unwrap();
        fs::write(seed.join("src/main.rs"), "fn main() { println!(\"{}:{}:{}\", probe_dep::value(), env!(\"GENERATED_VALUE\"), \"A\"); }\n").unwrap();
        fs::write(seed.join("input.txt"), "one").unwrap();
        fs::write(seed.join("build.rs"), "fn main() { println!(\"cargo:rerun-if-changed=input.txt\"); println!(\"cargo:rerun-if-env-changed=SCOPE_CACHE_PROBE_ENV\"); println!(\"cargo:rustc-env=GENERATED_VALUE={}\", std::fs::read_to_string(\"input.txt\").unwrap()); }\n").unwrap();
        run(Command::new("cargo")
            .current_dir(&seed)
            .args(["generate-lockfile", "--offline"]));
        run(Command::new("git")
            .current_dir(&seed)
            .args(["init", "--quiet"]));
        run(Command::new("git").current_dir(&seed).args(["add", "."]));
        run(Command::new("git").current_dir(&seed).args([
            "-c",
            "user.name=Cache test",
            "-c",
            "user.email=cache@example.test",
            "commit",
            "--quiet",
            "-m",
            "Fixture",
        ]));
        let source = run(Command::new("git")
            .current_dir(&seed)
            .args(["rev-parse", "HEAD"]))
        .trim()
        .to_string();
        run(Command::new("git")
            .current_dir(&seed)
            .args(["bundle", "create"])
            .arg(&bundle)
            .arg("HEAD"));
        fs::File::create(&archive).unwrap();
        let fixture = Self {
            _temp: temp,
            workspace,
            target,
            bundle,
            archive,
            source,
        };
        fixture.checkout();
        fixture
    }

    fn checkout(&self) {
        if self.workspace.exists() {
            fs::remove_dir_all(&self.workspace).unwrap();
        }
        if self.target.exists() {
            fs::remove_dir_all(&self.target).unwrap();
        }
        fs::create_dir(&self.target).unwrap();
        crate::checkout::checkout_exact_commit(&self.bundle, &self.workspace, &self.source)
            .unwrap();
    }

    fn build(&self, environment: Option<(&str, &str)>) -> bool {
        let mut command = Command::new("cargo");
        command
            .current_dir(&self.workspace)
            .args(["build", "--offline", "--locked", "--message-format=json"])
            .env("CARGO_TARGET_DIR", &self.target)
            .env("CARGO_INCREMENTAL", "0")
            .env_remove("RUSTFLAGS")
            .env_remove("CARGO_ENCODED_RUSTFLAGS")
            .env_remove("SCOPE_CACHE_PROBE_ENV");
        if let Some((name, value)) = environment {
            command.env(name, value);
        }
        let output = run(&mut command);
        let artifacts: Vec<_> = output
            .lines()
            .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
            .filter(|entry| entry["reason"] == "compiler-artifact")
            .collect();
        assert!(!artifacts.is_empty());
        artifacts.iter().all(|entry| entry["fresh"] == true)
    }

    fn output(&self) -> String {
        run(&mut Command::new(self.target.join("debug/cache-probe")))
            .trim()
            .to_string()
    }

    fn snapshot(&self) -> SourceSnapshot {
        SourceSnapshot::capture(&self.workspace).unwrap()
    }

    fn save(&self, snapshot: &SourceSnapshot) {
        create_archive(&self.target, &self.archive, Some(snapshot)).unwrap();
    }

    fn restore(&self) -> SourceSnapshot {
        let mut snapshot = self.snapshot();
        let restored = extract_archive(&self.archive, &self.target, Some(&self.workspace)).unwrap();
        snapshot.restore(&[restored]).unwrap();
        snapshot
    }

    fn replace(&self, path: &str, before: &str, after: &str) {
        let path = self.workspace.join(path);
        let source = fs::read_to_string(&path).unwrap();
        assert!(source.contains(before));
        fs::write(path, source.replace(before, after)).unwrap();
    }

    fn commit_current(&mut self) {
        run(Command::new("git")
            .current_dir(&self.workspace)
            .args(["add", "."]));
        run(Command::new("git").current_dir(&self.workspace).args([
            "-c",
            "user.name=Cache test",
            "-c",
            "user.email=cache@example.test",
            "commit",
            "--quiet",
            "-m",
            "Changed source",
        ]));
        self.source = run(Command::new("git")
            .current_dir(&self.workspace)
            .args(["rev-parse", "HEAD"]))
        .trim()
        .to_string();
        fs::remove_file(&self.bundle).unwrap();
        run(Command::new("git")
            .current_dir(&self.workspace)
            .args(["bundle", "create"])
            .arg(&self.bundle)
            .arg("HEAD"));
    }
}

fn run(command: &mut Command) -> String {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "{command:?}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn real_cargo_cache_reuses_unchanged_checkout_and_rebuilds_changed_inputs() {
    let fixture = Fixture::new();
    let snapshot = fixture.snapshot();
    assert!(!fixture.build(None));
    assert_eq!(fixture.output(), "1:one:A");
    fixture.save(&snapshot);

    fixture.checkout();
    extract_archive(&fixture.archive, &fixture.target, Some(&fixture.workspace)).unwrap();
    assert!(
        !fixture.build(None),
        "output timestamps alone must not certify a new checkout"
    );

    fixture.checkout();
    fixture.restore();
    assert!(
        fixture.build(None),
        "unchanged checkout should reuse every compiler artifact"
    );
    assert_eq!(fixture.output(), "1:one:A");

    for (case, expected) in [
        ("same-size-source", "1:one:B"),
        ("dependency", "2:one:A"),
        ("build-input", "1:two:A"),
        ("build-script", "1:script:A"),
        ("manifest", "1:one:A"),
        ("rustflags", "1:one:A"),
        ("build-env", "1:one:A"),
    ] {
        fixture.checkout();
        let environment = match case {
            "same-size-source" => {
                fixture.replace("src/main.rs", "\"A\"", "\"B\"");
                None
            }
            "dependency" => {
                fixture.replace("dep/src/lib.rs", "{ 1 }", "{ 2 }");
                None
            }
            "build-input" => {
                fixture.replace("input.txt", "one", "two");
                None
            }
            "build-script" => {
                fs::write(
                    fixture.workspace.join("build.rs"),
                    "fn main() { println!(\"cargo:rustc-env=GENERATED_VALUE=script\"); }\n",
                )
                .unwrap();
                None
            }
            "manifest" => {
                fixture.replace("Cargo.toml", "edition = '2024'", "edition = '2021'");
                None
            }
            "rustflags" => Some(("RUSTFLAGS", "-Copt-level=1")),
            "build-env" => Some(("SCOPE_CACHE_PROBE_ENV", "changed")),
            _ => unreachable!(),
        };
        // Deliberately stale mtimes cannot disguise changed file contents.
        for path in [
            "src/main.rs",
            "dep/src/lib.rs",
            "input.txt",
            "build.rs",
            "Cargo.toml",
        ] {
            set_modified(
                &fixture.workspace,
                Path::new(path),
                SystemTime::UNIX_EPOCH + Duration::from_secs(2),
            )
            .unwrap();
        }
        fixture.restore();
        assert!(!fixture.build(environment), "{case} must rebuild");
        assert_eq!(
            fixture.output(),
            expected,
            "{case} must execute the correct binary"
        );
    }
}

#[test]
fn inputs_changed_during_the_job_are_not_certified_as_unchanged() {
    let fixture = Fixture::new();
    let snapshot = fixture.snapshot();
    fixture.replace("input.txt", "one", "two");
    assert!(!fixture.build(None));
    assert_eq!(fixture.output(), "1:two:A");
    // Even reverting the contents after compiling must not bless the binary.
    fixture.replace("input.txt", "two", "one");
    fixture.save(&snapshot);
    fixture.checkout();
    fixture.restore();
    assert!(!fixture.build(None));
    assert_eq!(fixture.output(), "1:one:A");
}

#[test]
fn changed_source_cache_is_fresh_on_the_next_exact_checkout() {
    let mut fixture = Fixture::new();
    let snapshot = fixture.snapshot();
    assert!(!fixture.build(None));
    fixture.save(&snapshot);
    fixture.checkout();
    fixture.replace("src/main.rs", "\"A\"", "\"B\"");
    fixture.commit_current();
    let changed_snapshot = fixture.restore();
    assert!(!fixture.build(None));
    assert_eq!(fixture.output(), "1:one:B");
    fixture.save(&changed_snapshot);
    fixture.checkout();
    fixture.restore();
    assert!(fixture.build(None));
    assert_eq!(fixture.output(), "1:one:B");
}

#[test]
fn new_files_and_disagreeing_source_caches_stay_newer_than_outputs() {
    let fixture = Fixture::new();
    let first_sources = fixture.snapshot();
    fs::write(fixture.target.join("output"), "first cache").unwrap();
    fixture.save(&first_sources);
    let first_archive =
        extract_archive(&fixture.archive, &fixture.target, Some(&fixture.workspace)).unwrap();

    fixture.replace("input.txt", "one", "two");
    let second_sources = fixture.snapshot();
    fixture.save(&second_sources);
    let second_archive =
        extract_archive(&fixture.archive, &fixture.target, Some(&fixture.workspace)).unwrap();
    let newest_output = first_archive
        .newest_output
        .max(second_archive.newest_output);
    fs::write(fixture.workspace.join("new.txt"), "new input").unwrap();
    run(Command::new("git")
        .current_dir(&fixture.workspace)
        .args(["add", "new.txt"]));
    let mut current = fixture.snapshot();
    current.restore(&[first_archive, second_archive]).unwrap();
    for path in ["input.txt", "new.txt"] {
        assert!(
            fs::metadata(fixture.workspace.join(path))
                .unwrap()
                .modified()
                .unwrap()
                > newest_output
        );
    }
}
