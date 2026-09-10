use scope_domain::{
    content::is_supported_git_file_mode, content_ref::ContentRef, repository::git::GitPackSpan,
};
use scope_git_process::{ProcessCancellation, ProcessLimits, run_cancellable};
use scope_git_storage::GitSegmentStore;
use scope_object_store::{ObjectStore, source_blob_bytes_bounded};
use scope_postgres::db::{DependencyAnalysisClaim, DependencySnapshotFile};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{Duration, Instant},
};
use tempfile::TempDir;

const MAX_FILES: usize = 20_000;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_GIT_PACK_SPANS: usize = 128;
const MAX_GIT_PACK_BYTES: u64 = 256 * 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

pub(super) struct Snapshot {
    directory: TempDir,
    pub(super) source: PathBuf,
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = set_tree_writable(self.directory.path());
    }
}

pub(super) async fn materialize(
    claim: &DependencyAnalysisClaim,
    objects: Arc<dyn ObjectStore>,
    segments: Arc<GitSegmentStore>,
    data_dir: &Path,
    cancellation: ProcessCancellation,
) -> anyhow::Result<Snapshot> {
    check_cancellation(&cancellation)?;
    validate_files(&claim.files)?;
    let scratch = data_dir.join("dependency-checks");
    fs::create_dir_all(&scratch)?;
    let directory = tempfile::Builder::new()
        .prefix("snapshot-")
        .tempdir_in(scratch)?;
    let source = directory.path().join("source");
    fs::create_dir(&source)?;
    let snapshot = Snapshot { directory, source };
    let bare = snapshot.directory.path().join("git");
    if validate_git_restore(&claim.files, &claim.git_pack_spans)? {
        let deadline = Instant::now() + GIT_TIMEOUT;
        git(
            None,
            &["init", "--bare", "--template=", &bare.to_string_lossy()],
            None,
            64 * 1024,
            remaining_git_time(deadline)?,
            &cancellation,
        )?;
        for span in &claim.git_pack_spans {
            check_cancellation(&cancellation)?;
            crate::git_repo::index_git_segment(
                segments.as_ref(),
                claim.incarnation.repository_id(),
                &span.segment,
                &bare,
                remaining_git_time(deadline)?,
                Some(&cancellation),
            )
            .await?;
        }
    }
    let files = claim.files.clone();
    tokio::task::spawn_blocking(move || {
        write_files(
            &snapshot.source,
            &bare,
            &files,
            objects.as_ref(),
            &cancellation,
        )?;
        freeze_tree(&snapshot.source)?;
        Ok(snapshot)
    })
    .await?
}

fn check_cancellation(cancellation: &ProcessCancellation) -> anyhow::Result<()> {
    if cancellation.is_cancelled() {
        anyhow::bail!("dependency snapshot was canceled");
    }
    Ok(())
}

fn validate_git_restore(
    files: &[DependencySnapshotFile],
    spans: &[GitPackSpan],
) -> anyhow::Result<bool> {
    let required = files.iter().any(|file| {
        needs_content(file.path.as_str())
            && matches!(&file.blob.content_ref, ContentRef::GitBlob { .. })
    });
    if !required {
        return Ok(false);
    }
    if spans.len() > MAX_GIT_PACK_SPANS {
        anyhow::bail!("dependency snapshot exceeds {MAX_GIT_PACK_SPANS} Git pack spans");
    }
    let bytes = spans.iter().try_fold(0_u64, |total, span| {
        total.checked_add(span.segment.plaintext_bytes)
    });
    if bytes.is_none_or(|bytes| bytes > MAX_GIT_PACK_BYTES) {
        anyhow::bail!("dependency snapshot exceeds {MAX_GIT_PACK_BYTES} Git pack bytes");
    }
    Ok(true)
}

fn remaining_git_time(deadline: Instant) -> anyhow::Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| anyhow::anyhow!("dependency snapshot Git restoration timed out"))
}

fn validate_files(files: &[DependencySnapshotFile]) -> anyhow::Result<()> {
    if files.len() > MAX_FILES {
        anyhow::bail!("dependency snapshot exceeds {MAX_FILES} files");
    }
    let mut bytes = 0_u64;
    for file in files {
        if !is_supported_git_file_mode(&file.blob.git_file_mode) {
            anyhow::bail!("dependency snapshot contains an unsupported file mode");
        }
        if needs_content(file.path.as_str()) {
            if file.blob.size_bytes > MAX_FILE_BYTES as u64 {
                anyhow::bail!("dependency source file exceeds {MAX_FILE_BYTES} bytes");
            }
            bytes = bytes.saturating_add(file.blob.size_bytes);
        }
    }
    if bytes > MAX_SOURCE_BYTES as u64 {
        anyhow::bail!("dependency snapshot exceeds {MAX_SOURCE_BYTES} source bytes");
    }
    Ok(())
}

// Resolvers need asset paths, but parsing never needs binary asset contents.
// Empty placeholders preserve file existence without reading large media blobs.
fn needs_content(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension,
                "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "json" | "jsonc"
            )
        })
}

fn write_files(
    source: &Path,
    bare: &Path,
    files: &[DependencySnapshotFile],
    objects: &dyn ObjectStore,
    cancellation: &ProcessCancellation,
) -> anyhow::Result<()> {
    check_cancellation(cancellation)?;
    let git_blobs = read_git_blobs(bare, files, cancellation)?;
    for file in files {
        check_cancellation(cancellation)?;
        let path = source.join(file.path.as_str().trim_start_matches('/'));
        let parent = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("snapshot file has no parent"))?;
        fs::create_dir_all(parent)?;
        if !needs_content(file.path.as_str()) {
            fs::write(path, [])?;
            continue;
        }
        match &file.blob.content_ref {
            ContentRef::GitBlob { git_oid } => {
                if git_oid != &file.blob.git_oid {
                    anyhow::bail!("dependency source blob identity does not match its Git OID");
                }
                let content = git_blobs
                    .get(git_oid)
                    .ok_or_else(|| anyhow::anyhow!("missing dependency Git blob"))?;
                if content.len() as u64 != file.blob.size_bytes {
                    anyhow::bail!("dependency source blob size does not match metadata");
                }
                fs::write(path, content)?;
            }
            _ => fs::write(
                path,
                source_blob_bytes_bounded(objects, &file.blob, MAX_FILE_BYTES)?,
            )?,
        }
    }
    Ok(())
}

fn read_git_blobs(
    bare: &Path,
    files: &[DependencySnapshotFile],
    cancellation: &ProcessCancellation,
) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let expected: BTreeMap<_, _> = files
        .iter()
        .filter(|file| needs_content(file.path.as_str()))
        .filter_map(|file| match &file.blob.content_ref {
            ContentRef::GitBlob { git_oid } => Some((git_oid.clone(), file.blob.size_bytes)),
            _ => None,
        })
        .collect();
    if expected.is_empty() {
        return Ok(BTreeMap::new());
    }
    for oid in expected.keys() {
        if oid.len() != 40 || !oid.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            anyhow::bail!("dependency source has an invalid Git OID");
        }
    }
    let input = expected
        .keys()
        .map(|oid| format!("{oid}\n"))
        .collect::<String>()
        .into_bytes();
    let output = git(
        Some(bare),
        &["cat-file", "--batch"],
        Some(input),
        MAX_SOURCE_BYTES + MAX_FILES * 100,
        GIT_TIMEOUT,
        cancellation,
    )?;
    parse_git_blobs(&output, expected)
}

fn parse_git_blobs(
    output: &[u8],
    expected: BTreeMap<String, u64>,
) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut remaining = output;
    let mut blobs = BTreeMap::new();
    for (oid, size) in expected {
        let header_end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .ok_or_else(|| anyhow::anyhow!("Git blob batch response has no header"))?;
        let header = std::str::from_utf8(&remaining[..header_end])?;
        if header != format!("{oid} blob {size}") {
            anyhow::bail!("Git blob batch response does not match source metadata");
        }
        remaining = &remaining[header_end + 1..];
        let size = usize::try_from(size)?;
        if remaining.get(size) != Some(&b'\n') {
            anyhow::bail!("Git blob batch response is truncated");
        }
        blobs.insert(oid, remaining[..size].to_vec());
        remaining = &remaining[size + 1..];
    }
    if !remaining.is_empty() {
        anyhow::bail!("unexpected trailing Git batch content");
    }
    Ok(blobs)
}

fn git(
    repo: Option<&Path>,
    args: &[&str],
    input: Option<Vec<u8>>,
    limit: usize,
    timeout: Duration,
    cancellation: &ProcessCancellation,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command.arg("-c").arg("core.hooksPath=/dev/null");
    if let Some(repo) = repo {
        command.arg("--git-dir").arg(repo);
    }
    command.args(args);
    let output = run_cancellable(
        &mut command,
        input,
        ProcessLimits::new(timeout).with_max_stdout_bytes(limit),
        "materializing dependency snapshot",
        cancellation,
    )?;
    if !output.status.success() {
        anyhow::bail!(
            "dependency snapshot Git operation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

fn freeze_tree(path: &Path) -> std::io::Result<()> {
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            freeze_tree(&entry?.path())?;
        }
    }
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(true);
    fs::set_permissions(path, permissions)
}

fn set_tree_writable(path: &Path) -> std::io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(if path.is_dir() { 0o700 } else { 0o600 });
    }
    #[cfg(not(unix))]
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions)?;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            set_tree_writable(&entry?.path())?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use scope_domain::{content::SourceBlob, policy::ScopePath, repository::git::GitSegmentRef};
    use scope_object_store::{MemoryObjectStore, put_source_blob};

    fn snapshot_file(path: &str, size_bytes: u64, git_file_mode: &str) -> DependencySnapshotFile {
        DependencySnapshotFile {
            path: ScopePath::parse(format!("/{path}")).unwrap(),
            blob: SourceBlob {
                content_ref: ContentRef::blob_sha256("a".repeat(64)),
                sha256: "a".repeat(64),
                git_oid: "b".repeat(40),
                git_file_mode: git_file_mode.to_string(),
                size_bytes,
            },
        }
    }

    fn git_snapshot_file(path: &str) -> DependencySnapshotFile {
        let mut file = snapshot_file(path, 1, "100644");
        file.blob.content_ref = ContentRef::git_blob(file.blob.git_oid.clone());
        file
    }

    fn git_span(plaintext_bytes: u64) -> GitPackSpan {
        GitPackSpan {
            first_sequence: 1,
            last_sequence: 1,
            geometric_tier: 0,
            base_oid: None,
            head_oid: "b".repeat(40),
            segment: GitSegmentRef {
                segment_id: "segment".to_string(),
                sha256: "c".repeat(64),
                plaintext_bytes,
                encoding_version: 2,
            },
        }
    }

    #[test]
    fn batch_parser_preserves_embedded_newlines_and_rejects_wrong_identity() {
        let oid = "a".repeat(40);
        let expected = BTreeMap::from([(oid.clone(), 3)]);
        let bytes = format!("{oid} blob 3\na\nb\n").into_bytes();
        assert_eq!(
            parse_git_blobs(&bytes, expected.clone()).unwrap()[&oid],
            b"a\nb"
        );
        assert!(parse_git_blobs(b"wrong blob 3\na\nb\n", expected.clone()).is_err());
        assert!(parse_git_blobs(&bytes[..bytes.len() - 1], expected).is_err());
    }

    #[test]
    fn source_and_resolution_configs_are_materialized_but_binary_assets_are_placeholders() {
        assert!(needs_content("src/module.mts"));
        assert!(needs_content("packages/app/tsconfig.json"));
        assert!(needs_content("packages/app/package.json"));
        assert!(!needs_content("assets/large.mp4"));
        assert!(!needs_content("src/unsupported.rs"));
    }

    #[test]
    fn snapshot_validation_rejects_unsupported_modes_and_each_resource_limit() {
        for mode in ["040000", "120000", "160000"] {
            let unsupported_mode = snapshot_file("src/main.ts", 1, mode);
            assert!(
                validate_files(&[unsupported_mode])
                    .unwrap_err()
                    .to_string()
                    .contains("unsupported file mode")
            );
        }

        let too_many = vec![snapshot_file("asset.bin", 0, "100644"); MAX_FILES + 1];
        assert!(validate_files(&too_many[..MAX_FILES]).is_ok());
        assert!(
            validate_files(&too_many)
                .unwrap_err()
                .to_string()
                .contains("exceeds 20000 files")
        );

        let oversized_file = snapshot_file("src/main.ts", MAX_FILE_BYTES as u64 + 1, "100644");
        assert!(
            validate_files(&[snapshot_file(
                "src/main.ts",
                MAX_FILE_BYTES as u64,
                "100644"
            )])
            .is_ok()
        );
        assert!(
            validate_files(&[oversized_file])
                .unwrap_err()
                .to_string()
                .contains("source file exceeds 4194304 bytes")
        );

        let total_limit_files = (0..=(MAX_SOURCE_BYTES / MAX_FILE_BYTES))
            .map(|index| snapshot_file(&format!("src/{index}.ts"), MAX_FILE_BYTES as u64, "100644"))
            .collect::<Vec<_>>();
        assert!(validate_files(&total_limit_files[..MAX_SOURCE_BYTES / MAX_FILE_BYTES]).is_ok());
        assert!(
            validate_files(&total_limit_files)
                .unwrap_err()
                .to_string()
                .contains("exceeds 67108864 source bytes")
        );
    }

    #[test]
    fn git_restore_is_bounded_only_when_an_analyzer_input_needs_a_git_blob() {
        let exact_count = vec![git_span(1); MAX_GIT_PACK_SPANS];
        assert!(validate_git_restore(&[git_snapshot_file("src/main.ts")], &exact_count).unwrap());
        let over_count = vec![git_span(1); MAX_GIT_PACK_SPANS + 1];
        assert!(
            validate_git_restore(&[git_snapshot_file("src/main.ts")], &over_count)
                .unwrap_err()
                .to_string()
                .contains("exceeds 128 Git pack spans")
        );

        assert!(
            validate_git_restore(
                &[git_snapshot_file("src/main.ts")],
                &[git_span(MAX_GIT_PACK_BYTES)]
            )
            .unwrap()
        );
        assert!(
            validate_git_restore(
                &[git_snapshot_file("src/main.ts")],
                &[git_span(MAX_GIT_PACK_BYTES + 1)]
            )
            .unwrap_err()
            .to_string()
            .contains("exceeds 268435456 Git pack bytes")
        );

        assert!(
            !validate_git_restore(&[snapshot_file("src/main.ts", 1, "100644")], &over_count)
                .unwrap()
        );
        assert!(
            !validate_git_restore(&[git_snapshot_file("assets/logo.png")], &over_count).unwrap()
        );
    }

    #[test]
    fn blob_snapshot_preserves_analyzer_inputs_and_cleans_up_readonly_placeholders() {
        let objects = MemoryObjectStore::new();
        let files = [
            (
                "tsconfig.json",
                br#"{"compilerOptions":{"baseUrl":"."}}"#.as_slice(),
            ),
            ("src/main.ts", b"import '../data.json';\n".as_slice()),
            ("data.json", br#"{"scope":true}"#.as_slice()),
            (
                "internal/private.rs",
                b"pub const PRIVATE: bool = true;\n".as_slice(),
            ),
            ("assets/logo.png", b"not really a png".as_slice()),
        ]
        .into_iter()
        .map(|(path, bytes)| DependencySnapshotFile {
            path: ScopePath::parse(format!("/{path}")).unwrap(),
            blob: put_source_blob(&objects, bytes).unwrap(),
        })
        .collect::<Vec<_>>();

        assert!(
            files
                .iter()
                .all(|file| matches!(&file.blob.content_ref, ContentRef::BlobSha256(_)))
        );

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("source");
        fs::create_dir(&source).unwrap();
        let snapshot = Snapshot { directory, source };
        let snapshot_root = snapshot.directory.path().to_path_buf();

        write_files(
            &snapshot.source,
            &snapshot.directory.path().join("unused-git"),
            &files,
            &objects,
            &ProcessCancellation::new(),
        )
        .unwrap();
        freeze_tree(&snapshot.source).unwrap();

        assert_eq!(
            fs::read(snapshot.source.join("tsconfig.json")).unwrap(),
            br#"{"compilerOptions":{"baseUrl":"."}}"#
        );
        assert_eq!(
            fs::read(snapshot.source.join("src/main.ts")).unwrap(),
            b"import '../data.json';\n"
        );
        assert_eq!(
            fs::read(snapshot.source.join("data.json")).unwrap(),
            br#"{"scope":true}"#
        );
        assert_eq!(
            fs::read(snapshot.source.join("internal/private.rs")).unwrap(),
            b""
        );
        assert_eq!(
            fs::read(snapshot.source.join("assets/logo.png")).unwrap(),
            b""
        );
        assert!(
            fs::metadata(&snapshot.source)
                .unwrap()
                .permissions()
                .readonly()
        );
        assert!(
            fs::metadata(snapshot.source.join("src/main.ts"))
                .unwrap()
                .permissions()
                .readonly()
        );

        drop(snapshot);
        assert!(!snapshot_root.exists());
    }
}
