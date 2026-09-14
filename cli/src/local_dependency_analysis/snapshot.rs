use super::check_cancelled;
use crate::progress::{CancellationToken, run_cancellable};
use anyhow::{Context, bail, ensure};
use std::{collections::BTreeMap, fs, path::Path, process::Command, time::Duration};
use tempfile::TempDir;

const MAX_FILES: usize = 20_000;
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 4096;
const GIT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug)]
struct File {
    path: String,
    oid: String,
    size: usize,
}

pub(super) fn validate_commit(commit: &str) -> anyhow::Result<()> {
    ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Dependency check requires the reviewed commit's full Git OID"
    );
    Ok(())
}

pub(super) fn materialize(
    repo: &Path,
    commit: &str,
    cancellation: &CancellationToken,
) -> anyhow::Result<TempDir> {
    validate_commit(commit)?;
    let tree = git(
        repo,
        &["ls-tree", "-rlz", "--full-tree", commit],
        None,
        MAX_FILES * (MAX_PATH_BYTES + 100),
        cancellation,
    )?;
    let files = parse_tree(&tree)?;
    let directory = tempfile::Builder::new()
        .prefix("scope-dependency-")
        .tempdir()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))?;
    }
    let expected: BTreeMap<_, _> = files
        .iter()
        .filter(|file| needs_content(&file.path))
        .map(|file| (file.oid.clone(), file.size))
        .collect();
    let blobs = if expected.is_empty() {
        BTreeMap::new()
    } else {
        let input = expected
            .keys()
            .map(|oid| format!("{oid}\n"))
            .collect::<String>()
            .into_bytes();
        let bytes = git(
            repo,
            &["cat-file", "--batch"],
            Some(input),
            MAX_SOURCE_BYTES + MAX_FILES * 100,
            cancellation,
        )?;
        parse_blobs(&bytes, &expected)?
    };
    for file in files {
        check_cancelled(cancellation)?;
        let path = directory.path().join(&file.path);
        fs::create_dir_all(path.parent().context("Snapshot path has no parent")?)?;
        // Asset paths participate in resolution without reading their contents.
        let bytes = if needs_content(&file.path) {
            blobs
                .get(&file.oid)
                .context("Snapshot Git blob is missing")?
                .as_slice()
        } else {
            &[]
        };
        fs::write(path, bytes)?;
    }
    Ok(directory)
}

fn parse_tree(bytes: &[u8]) -> anyhow::Result<Vec<File>> {
    let mut files = Vec::new();
    let mut source_bytes = 0usize;
    for record in bytes
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        ensure!(
            files.len() < MAX_FILES,
            "Dependency snapshot exceeds {MAX_FILES} files"
        );
        let separator = record
            .iter()
            .position(|byte| *byte == b'\t')
            .context("Git tree entry has no path")?;
        let header = std::str::from_utf8(&record[..separator])?
            .split_whitespace()
            .collect::<Vec<_>>();
        ensure!(header.len() == 4, "Invalid Git tree entry");
        ensure!(
            matches!(header[0], "100644" | "100755") && header[1] == "blob",
            "Dependency snapshot contains a symbolic link or submodule"
        );
        validate_commit(header[2])?;
        let path = std::str::from_utf8(&record[separator + 1..])
            .context("Dependency snapshot contains a non-UTF-8 path")?;
        validate_path(path)?;
        let size: usize = header[3].parse().context("Invalid Git blob size")?;
        if needs_content(path) {
            ensure!(
                size <= MAX_FILE_BYTES,
                "Dependency source exceeds {MAX_FILE_BYTES} bytes"
            );
            source_bytes = source_bytes
                .checked_add(size)
                .context("Dependency source size overflow")?;
            ensure!(
                source_bytes <= MAX_SOURCE_BYTES,
                "Dependency snapshot exceeds {MAX_SOURCE_BYTES} source bytes"
            );
        }
        files.push(File {
            path: path.to_owned(),
            oid: header[2].to_owned(),
            size,
        });
    }
    Ok(files)
}

fn validate_path(path: &str) -> anyhow::Result<()> {
    ensure!(
        !path.is_empty() && path.len() <= MAX_PATH_BYTES && !path.contains('\\'),
        "Unsupported dependency snapshot path"
    );
    for part in path.split('/') {
        ensure!(
            !part.is_empty() && !matches!(part, "." | "..") && !part.eq_ignore_ascii_case(".git"),
            "Unsafe dependency snapshot path"
        );
        #[cfg(windows)]
        ensure!(
            !part.contains(':') && !part.ends_with(['.', ' ']),
            "Unsupported Windows dependency snapshot path"
        );
    }
    Ok(())
}

fn needs_content(path: &str) -> bool {
    Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "js" | "jsx" | "mjs" | "cjs" | "ts" | "tsx" | "mts" | "cts" | "json" | "jsonc"
            )
        })
}

fn parse_blobs(
    bytes: &[u8],
    expected: &BTreeMap<String, usize>,
) -> anyhow::Result<BTreeMap<String, Vec<u8>>> {
    let mut remaining = bytes;
    let mut blobs = BTreeMap::new();
    for (oid, size) in expected {
        let end = remaining
            .iter()
            .position(|byte| *byte == b'\n')
            .context("Git blob batch has no header")?;
        ensure!(
            &remaining[..end] == format!("{oid} blob {size}").as_bytes(),
            "Git blob identity or size changed during snapshot"
        );
        remaining = &remaining[end + 1..];
        ensure!(
            remaining.get(*size) == Some(&b'\n'),
            "Git blob batch is truncated"
        );
        blobs.insert(oid.clone(), remaining[..*size].to_vec());
        remaining = &remaining[*size + 1..];
    }
    ensure!(remaining.is_empty(), "Unexpected trailing Git blob content");
    Ok(blobs)
}

pub(super) fn git(
    repo: &Path,
    args: &[&str],
    input: Option<Vec<u8>>,
    max_output: usize,
    cancellation: &CancellationToken,
) -> anyhow::Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .current_dir(repo)
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    let output = run_cancellable(&mut command, input, cancellation, GIT_TIMEOUT, max_output)
        .context("Read committed dependency snapshot")?;
    if !output.status.success() {
        bail!(
            "Read committed dependency snapshot: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_paths_cannot_escape_or_create_git_metadata() {
        for path in [
            "/absolute.ts",
            "../outside.ts",
            "a/../../b",
            "a\\b",
            ".git/config",
            "a/.GIT/config",
            "a//b",
            "./a",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        assert!(validate_path("packages/app/src/main.ts").is_ok());
    }
    #[test]
    fn batch_bytes_preserve_newlines_and_check_identity_and_size() {
        let oid = "a".repeat(40);
        let expected = BTreeMap::from([(oid.clone(), 3)]);
        let bytes = format!("{oid} blob 3\na\nb\n");
        assert_eq!(
            parse_blobs(bytes.as_bytes(), &expected).unwrap()[&oid],
            b"a\nb"
        );
        assert!(parse_blobs(b"wrong blob 3\na\nb\n", &expected).is_err());
        assert!(parse_blobs(&bytes.as_bytes()[..bytes.len() - 1], &expected).is_err());
    }
}
