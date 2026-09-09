use super::{files, sources::SourceSnapshot};
use anyhow::{Context as _, bail};
use scope_cache_domain::MAX_CACHE_OBJECT_BYTES;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    fs,
    io::{Read, Write},
    os::unix::fs::PermissionsExt as _,
    path::Path,
    time::{Duration, SystemTime},
};

const MAX_METADATA_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ArchiveMetadata {
    sources: Option<SourceSnapshot>,
}

pub(super) struct RestoredArchive {
    pub(super) sources: Option<SourceSnapshot>,
    pub(super) newest_output: SystemTime,
}

pub(super) fn reset_cache_directory(path: &Path) -> anyhow::Result<()> {
    fs::remove_dir_all(path)
        .with_context(|| format!("remove partial cache directory {}", path.display()))?;
    fs::create_dir_all(path).with_context(|| format!("recreate cache directory {}", path.display()))
}

pub(super) fn extract_archive(
    archive: &Path,
    destination: &Path,
    source_root: Option<&Path>,
) -> anyhow::Result<RestoredArchive> {
    let file = fs::File::open(archive)?;
    let mut decoder = zstd::Decoder::new(file).context("open compressed cache")?;
    // Frame runtime metadata before the tar stream so every payload name remains valid.
    let mut length = [0_u8; 8];
    decoder.read_exact(&mut length)?;
    let length = u64::from_be_bytes(length);
    if length > MAX_METADATA_BYTES {
        bail!("cache source metadata is too large");
    }
    let mut metadata = vec![0_u8; length as usize];
    decoder.read_exact(&mut metadata)?;
    let metadata: ArchiveMetadata = serde_json::from_slice(&metadata)?;
    let mut archive = tar::Archive::new(decoder);
    let mut directories = Vec::new();
    let mut newest_output = SystemTime::UNIX_EPOCH;
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        files::validate_relative(&path)?;
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink()) {
            bail!("unsupported cache archive entry type");
        }
        let modified = entry_modified(&mut entry)?;
        newest_output = newest_output.max(modified);
        if !entry
            .unpack_in(destination)
            .context("extract cache archive entry")?
        {
            bail!("cache archive entry escapes its destination");
        }
        if kind.is_dir() {
            directories.push((path, modified));
        } else {
            files::set_modified(destination, &path, modified)?;
        }
    }
    if let Some(sources) = &metadata.sources {
        sources.validate()?;
    }
    match (source_root, &metadata.sources) {
        (Some(root), Some(sources)) if root == sources.root => {}
        (None, None) => {}
        _ => bail!("cache source metadata does not match this workspace"),
    }
    // A clock-skewed cache could otherwise make newly generated build inputs
    // appear older than their cached outputs.
    if newest_output > SystemTime::now() {
        bail!("cache contains future output timestamps");
    }
    for (path, modified) in directories.into_iter().rev() {
        files::set_modified(destination, &path, modified)?;
    }
    Ok(RestoredArchive {
        sources: metadata.sources,
        newest_output,
    })
}

pub(super) fn create_archive(
    source: &Path,
    destination: &Path,
    sources: Option<&SourceSnapshot>,
) -> anyhow::Result<(u64, String)> {
    let file = fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(destination)?;
    let bounded = BoundedWriter::new(file, MAX_CACHE_OBJECT_BYTES);
    let mut encoder = zstd::Encoder::new(bounded, 3).context("create compressed cache")?;
    let metadata = serde_json::to_vec(&ArchiveMetadata {
        sources: sources.map(SourceSnapshot::for_save).transpose()?,
    })?;
    if metadata.len() as u64 > MAX_METADATA_BYTES {
        bail!("cache source metadata is too large");
    }
    encoder.write_all(&(metadata.len() as u64).to_be_bytes())?;
    encoder.write_all(&metadata)?;
    let mut archive = tar::Builder::new(encoder);
    append_directory_contents(&mut archive, source, Path::new(""))?;
    let encoder = archive.into_inner()?;
    let writer = encoder.finish()?;
    Ok(writer.identity())
}

fn append_directory_contents<W: Write>(
    archive: &mut tar::Builder<W>,
    source: &Path,
    relative: &Path,
) -> anyhow::Result<()> {
    let directory = source.join(relative);
    let mut entries = fs::read_dir(&directory)
        .with_context(|| format!("read cache directory {}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let relative_path = relative.join(entry.file_name());
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("read cache entry metadata {}", path.display()))?;
        let file_type = metadata.file_type();
        append_modified(archive, metadata.modified()?)?;
        if file_type.is_dir() {
            let mut header = normalized_header(tar::EntryType::Directory, 0, 0o755)?;
            append_entry(archive, &mut header, &relative_path, std::io::empty())?;
            append_directory_contents(archive, source, &relative_path)?;
        } else if file_type.is_file() {
            let mode = if metadata.permissions().mode() & 0o111 == 0 {
                0o644
            } else {
                0o755
            };
            let file = fs::File::open(&path)
                .with_context(|| format!("open cache entry {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Regular, metadata.len(), mode)?;
            append_entry(archive, &mut header, &relative_path, file)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path)
                .with_context(|| format!("read cache symlink {}", path.display()))?;
            let mut header = normalized_header(tar::EntryType::Symlink, 0, 0o777)?;
            archive
                .append_link(&mut header, &relative_path, target)
                .with_context(|| format!("archive cache symlink {}", relative_path.display()))?;
        } else {
            anyhow::bail!(
                "cache entry {} has an unsupported file type",
                path.display()
            );
        }
    }
    Ok(())
}

fn append_modified<W: Write>(
    archive: &mut tar::Builder<W>,
    modified: SystemTime,
) -> anyhow::Result<()> {
    let time = modified.duration_since(SystemTime::UNIX_EPOCH)?;
    let value = format!("{}.{:09}", time.as_secs(), time.subsec_nanos());
    archive.append_pax_extensions([("mtime", value.as_bytes())])?;
    Ok(())
}

fn entry_modified<R: Read>(entry: &mut tar::Entry<'_, R>) -> anyhow::Result<SystemTime> {
    let mut modified = None;
    for extension in entry
        .pax_extensions()?
        .context("cache timestamp is missing")?
    {
        let extension = extension?;
        if extension.key_bytes() != b"mtime" {
            continue;
        }
        if modified.is_some() {
            bail!("cache timestamp is duplicated");
        }
        let (seconds, nanos) = extension
            .value()?
            .split_once('.')
            .context("cache timestamp is invalid")?;
        if nanos.len() != 9 {
            bail!("cache timestamp precision is invalid");
        }
        let nanos = nanos.parse::<u32>()?;
        if nanos >= 1_000_000_000 {
            bail!("cache timestamp nanoseconds are invalid");
        }
        let duration = Duration::new(seconds.parse::<u64>()?, nanos);
        modified = Some(
            SystemTime::UNIX_EPOCH
                .checked_add(duration)
                .context("cache timestamp overflow")?,
        );
    }
    modified.context("cache timestamp is missing")
}

fn normalized_header(
    entry_type: tar::EntryType,
    size: u64,
    mode: u32,
) -> anyhow::Result<tar::Header> {
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(entry_type);
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_username("")?;
    header.set_groupname("")?;
    header.set_cksum();
    Ok(header)
}

fn append_entry<W: Write, R: Read>(
    archive: &mut tar::Builder<W>,
    header: &mut tar::Header,
    path: &Path,
    content: R,
) -> anyhow::Result<()> {
    archive
        .append_data(header, path, content)
        .with_context(|| format!("archive cache entry {}", path.display()))
}

pub(super) struct BoundedWriter<W> {
    inner: W,
    pub(super) written: u64,
    max_bytes: u64,
    hasher: Sha256,
}

impl<W> BoundedWriter<W> {
    pub(super) fn new(inner: W, max_bytes: u64) -> Self {
        Self {
            inner,
            written: 0,
            max_bytes,
            hasher: Sha256::new(),
        }
    }

    pub(super) fn identity(self) -> (u64, String) {
        (self.written, hex::encode(self.hasher.finalize()))
    }
}

impl<W: Write> Write for BoundedWriter<W> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.written.saturating_add(bytes.len() as u64) > self.max_bytes {
            return Err(std::io::Error::other(format!(
                "cache archive exceeds {} bytes",
                self.max_bytes
            )));
        }
        let written = self.inner.write(bytes)?;
        self.written += written as u64;
        self.hasher.update(&bytes[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
