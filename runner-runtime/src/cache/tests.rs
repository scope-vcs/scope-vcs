use super::{
    archive::{BoundedWriter, create_archive, extract_archive},
    files::set_modified,
    finalize::save_cache,
    identity::{MAX_CACHE_KEY_FILE_BYTES, digest_inputs_at, open_key_file},
    types::{CacheFinalizationOutcome, PreparedCache},
};
use crate::api::RuntimeClient;
use scope_domain::runs::cache::definition::CacheKeyInputs;
use std::{
    collections::BTreeMap,
    fs,
    io::{Read as _, Write as _},
    os::unix::fs::{PermissionsExt as _, symlink},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

#[test]
fn runtime_metadata_does_not_reserve_payload_names() {
    for directory in [false, true] {
        let source = tempfile::tempdir().unwrap();
        let relative = if directory {
            fs::create_dir(source.path().join(".scope-cache.json")).unwrap();
            ".scope-cache.json/content"
        } else {
            ".scope-cache.json"
        };
        fs::write(source.path().join(relative), "cached payload").unwrap();
        let archive = tempfile::NamedTempFile::new().unwrap();
        create_archive(source.path(), archive.path(), None).unwrap();
        let restored = tempfile::tempdir().unwrap();
        extract_archive(archive.path(), restored.path(), None).unwrap();
        assert_eq!(
            fs::read_to_string(restored.path().join(relative)).unwrap(),
            "cached payload"
        );
    }
}

#[test]
fn invalid_metadata_framing_is_rejected_before_extracting_payloads() {
    for length in [u64::MAX, 32 * 1024 * 1024 + 1, 16] {
        let archive = tempfile::NamedTempFile::new().unwrap();
        let mut encoder = zstd::Encoder::new(archive.reopen().unwrap(), 3).unwrap();
        encoder.write_all(&length.to_be_bytes()).unwrap();
        encoder.write_all(b"{}").unwrap();
        encoder.finish().unwrap();
        let restored = tempfile::tempdir().unwrap();
        assert!(extract_archive(archive.path(), restored.path(), None).is_err());
        assert_eq!(fs::read_dir(restored.path()).unwrap().count(), 0);
    }
}

#[test]
fn archives_are_identical_across_creation_order() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    populate_cache(first.path(), false, Duration::from_secs(10));
    populate_cache(second.path(), true, Duration::from_secs(10));
    let first_archive = tempfile::NamedTempFile::new().unwrap();
    let second_archive = tempfile::NamedTempFile::new().unwrap();

    let first_identity = create_archive(first.path(), first_archive.path(), None).unwrap();
    let second_identity = create_archive(second.path(), second_archive.path(), None).unwrap();

    let first_bytes = fs::read(first_archive.path()).unwrap();
    let second_bytes = fs::read(second_archive.path()).unwrap();
    assert_eq!(first_bytes, second_bytes);
    assert_eq!(first_identity, second_identity);
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        first_identity,
        (
            first_bytes.len() as u64,
            hex::encode(Sha256::digest(&first_bytes))
        )
    );
}

#[test]
fn archives_normalize_ownership_and_preserve_precise_timestamps() {
    let source = tempfile::tempdir().unwrap();
    let modified = Duration::new(30, 123_456_789);
    populate_cache(source.path(), true, modified);
    let output = tempfile::NamedTempFile::new().unwrap();
    create_archive(source.path(), output.path(), None).unwrap();

    let mut decoder = zstd::Decoder::new(fs::File::open(output.path()).unwrap()).unwrap();
    let mut length = [0_u8; 8];
    decoder.read_exact(&mut length).unwrap();
    let mut metadata = vec![0_u8; u64::from_be_bytes(length) as usize];
    decoder.read_exact(&mut metadata).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&metadata).unwrap(),
        serde_json::json!({"sources": null})
    );
    let mut archive = tar::Archive::new(decoder);
    let headers = archive
        .entries()
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            let header = entry.header();
            (
                entry.path().unwrap().into_owned(),
                header.mode().unwrap(),
                header.uid().unwrap(),
                header.gid().unwrap(),
                header.mtime().unwrap(),
            )
        })
        .collect::<Vec<_>>();

    let paths = headers
        .iter()
        .map(|(path, ..)| path.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        ["bin", "bin/run", "data.txt", "run-link"]
            .map(PathBuf::from)
            .to_vec()
    );
    assert!(
        headers
            .iter()
            .all(|(_, _, uid, gid, mtime)| (*uid, *gid, *mtime) == (0, 0, 0))
    );
    let modes = headers
        .into_iter()
        .map(|(path, mode, ..)| (path, mode))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(modes[Path::new("bin")], 0o755);
    assert_eq!(modes[Path::new("bin/run")], 0o755);
    assert_eq!(modes[Path::new("data.txt")], 0o644);
    assert_eq!(modes[Path::new("run-link")], 0o777);
    let restored = tempfile::tempdir().unwrap();
    extract_archive(output.path(), restored.path(), None).unwrap();
    assert!(!restored.path().join(".scope-cache.json").exists());
    for path in ["bin", "bin/run", "data.txt", "run-link"] {
        let metadata = fs::symlink_metadata(restored.path().join(path)).unwrap();
        assert_eq!(
            metadata.modified().unwrap(),
            SystemTime::UNIX_EPOCH + modified
        );
    }
    assert_eq!(
        fs::read_link(restored.path().join("run-link")).unwrap(),
        Path::new("bin/run")
    );
}

#[test]
fn output_timestamps_are_part_of_the_cache_object_identity() {
    let source = tempfile::tempdir().unwrap();
    populate_cache(source.path(), false, Duration::from_secs(10));
    let output = tempfile::NamedTempFile::new().unwrap();
    let before = create_archive(source.path(), output.path(), None).unwrap();
    set_modified(
        source.path(),
        Path::new("data.txt"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(11),
    )
    .unwrap();
    let after = create_archive(source.path(), output.path(), None).unwrap();
    assert_ne!(before, after);
}

#[test]
fn future_outputs_and_workspace_mismatches_are_rejected() {
    let source = tempfile::tempdir().unwrap();
    populate_cache(source.path(), false, Duration::from_secs(10));
    let output = tempfile::NamedTempFile::new().unwrap();
    let restored = tempfile::tempdir().unwrap();
    create_archive(source.path(), output.path(), None).unwrap();
    assert!(
        extract_archive(
            output.path(),
            restored.path(),
            Some(Path::new("/workspace"))
        )
        .is_err()
    );
    set_modified(
        source.path(),
        Path::new("data.txt"),
        SystemTime::now() + Duration::from_secs(60),
    )
    .unwrap();
    create_archive(source.path(), output.path(), None).unwrap();
    assert!(extract_archive(output.path(), restored.path(), None).is_err());
}

#[test]
fn timestamp_restoration_never_follows_symlinks_outside_its_root() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("file"), "outside").unwrap();
    let before = fs::metadata(outside.path().join("file"))
        .unwrap()
        .modified()
        .unwrap();
    symlink(outside.path(), root.path().join("parent")).unwrap();
    assert!(
        set_modified(
            root.path(),
            Path::new("parent/file"),
            SystemTime::UNIX_EPOCH
        )
        .is_err()
    );
    symlink(outside.path().join("file"), root.path().join("link")).unwrap();
    set_modified(root.path(), Path::new("link"), SystemTime::UNIX_EPOCH).unwrap();
    assert_eq!(
        fs::metadata(outside.path().join("file"))
            .unwrap()
            .modified()
            .unwrap(),
        before
    );
}

#[test]
fn bounded_writer_accepts_the_limit_and_rejects_the_next_byte() {
    let mut writer = BoundedWriter::new(Vec::new(), 4);
    writer.write_all(b"four").unwrap();
    let error = writer.write_all(b"!").unwrap_err();
    assert_eq!(error.to_string(), "cache archive exceeds 4 bytes");
    assert_eq!(writer.written, 4);
}

#[test]
fn archive_hash_counts_only_bytes_accepted_by_partial_writes() {
    struct PartialWriter;
    impl std::io::Write for PartialWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len().min(2))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = BoundedWriter::new(PartialWriter, 8);
    writer.write_all(b"complete").unwrap();
    use sha2::{Digest as _, Sha256};
    assert_eq!(
        writer.identity(),
        (8, hex::encode(Sha256::digest(b"complete")))
    );
}

#[test]
fn exact_hit_skips_archive_hash_and_upload() {
    let cache = PreparedCache {
        exact_digest: "a".repeat(64),
        compatibility_group_digest: "b".repeat(64),
        path: PathBuf::from("/path/that/does/not/exist"),
        exact_hit: true,
        sources: None,
    };

    // The client cannot reach any service, so an upload attempt would be Skipped.
    assert!(matches!(
        save_cache(&RuntimeClient::disconnected_for_cache_tests(), &cache),
        CacheFinalizationOutcome::Ready
    ));
}

#[test]
fn cache_input_digest_distinguishes_missing_empty_content_and_environment() {
    let root = tempfile::tempdir().unwrap();
    let inputs = CacheKeyInputs::new(
        vec!["Cargo.lock".to_string()],
        vec!["RUSTUP_TOOLCHAIN".to_string()],
        false,
    )
    .unwrap();
    let mut environment = BTreeMap::from([("RUSTUP_TOOLCHAIN".to_string(), "1.98.0".to_string())]);
    let missing = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    fs::write(root.path().join("Cargo.lock"), []).unwrap();
    let empty = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    fs::write(root.path().join("Cargo.lock"), b"lock").unwrap();
    let content = digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();
    environment.insert("RUSTUP_TOOLCHAIN".to_string(), "1.99.0".to_string());
    let environment_changed =
        digest_inputs_at(&inputs, &environment, root.path(), "source-a").unwrap();

    let source_inputs = CacheKeyInputs::new(vec![], vec![], true).unwrap();
    let source_a = digest_inputs_at(&source_inputs, &environment, root.path(), "source-a").unwrap();
    let source_b = digest_inputs_at(&source_inputs, &environment, root.path(), "source-b").unwrap();

    assert_ne!(missing, empty);
    assert_ne!(empty, content);
    assert_ne!(content, environment_changed);
    assert_ne!(source_a, source_b);
    let other_workspace = tempfile::tempdir().unwrap();
    assert_ne!(
        source_a,
        digest_inputs_at(
            &source_inputs,
            &environment,
            other_workspace.path(),
            "source-a"
        )
        .unwrap()
    );
}

#[test]
fn cache_input_hashing_rejects_symlinks_and_directories() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("outside"), b"secret").unwrap();
    symlink("outside", root.path().join("linked")).unwrap();
    fs::create_dir(root.path().join("directory")).unwrap();
    fs::create_dir(root.path().join("outside-directory")).unwrap();
    fs::write(root.path().join("outside-directory/input"), b"secret").unwrap();
    symlink("outside-directory", root.path().join("linked-directory")).unwrap();
    assert!(open_key_file(root.path(), "linked").is_err());
    assert!(open_key_file(root.path(), "directory").is_err());
    assert!(open_key_file(root.path(), "linked-directory/input").is_err());

    let oversized = fs::File::create(root.path().join("oversized")).unwrap();
    oversized.set_len(MAX_CACHE_KEY_FILE_BYTES + 1).unwrap();
    assert!(open_key_file(root.path(), "oversized").is_err());
}

fn populate_cache(root: &Path, reverse: bool, modified_offset: Duration) {
    let files = if reverse {
        [("data.txt", "data"), ("bin/run", "#!/bin/sh\n")]
    } else {
        [("bin/run", "#!/bin/sh\n"), ("data.txt", "data")]
    };
    for (path, contents) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();
        let mode = if path.ends_with("run") { 0o755 } else { 0o644 };
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(SystemTime::UNIX_EPOCH + modified_offset))
            .unwrap();
    }
    symlink("bin/run", root.join("run-link")).unwrap();
    for path in ["bin", "run-link"] {
        set_modified(
            root,
            Path::new(path),
            SystemTime::UNIX_EPOCH + modified_offset,
        )
        .unwrap();
    }
}
