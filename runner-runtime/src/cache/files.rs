use anyhow::{Context as _, bail};
use std::{
    fs,
    os::unix::fs::OpenOptionsExt as _,
    path::{Component, Path},
    time::SystemTime,
};

pub(super) fn validate_relative(path: &Path) -> anyhow::Result<()> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!("cache metadata path must be relative: {}", path.display());
    }
    Ok(())
}

/// Inspect a cache or checkout entry without traversing a symlinked directory.
pub(super) fn inspect(root: &Path, relative: &Path) -> anyhow::Result<Option<fs::Metadata>> {
    validate_relative(relative)?;
    let mut path = root.to_path_buf();
    let mut parts = relative.components().peekable();
    while let Some(part) = parts.next() {
        path.push(part);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error).context("inspect cached file"),
        };
        if parts.peek().is_none() {
            return Ok(Some(metadata));
        }
        if !metadata.is_dir() {
            bail!(
                "cache metadata traverses a non-directory: {}",
                path.display()
            );
        }
    }
    unreachable!("relative paths are nonempty")
}

pub(super) fn open_file(path: &Path) -> anyhow::Result<fs::File> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("open cache input {}", path.display()))?;
    if !file.metadata()?.is_file() {
        bail!("cache input is not a regular file: {}", path.display());
    }
    Ok(file)
}

pub(super) fn set_modified(
    root: &Path,
    relative: &Path,
    modified: SystemTime,
) -> anyhow::Result<()> {
    let metadata = inspect(root, relative)?.context("cached timestamp entry is missing")?;
    let path = root.join(relative);
    if metadata.is_symlink() {
        filetime::set_symlink_file_times(
            path,
            filetime::FileTime::from_last_access_time(&metadata),
            filetime::FileTime::from_system_time(modified),
        )?;
    } else {
        let file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&path)?;
        if !(file.metadata()?.is_file() || file.metadata()?.is_dir()) {
            bail!("unsupported timestamp entry: {}", path.display());
        }
        file.set_times(fs::FileTimes::new().set_modified(modified))?;
    }
    Ok(())
}
