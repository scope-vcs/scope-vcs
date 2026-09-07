use super::{archive::RestoredArchive, files};
use anyhow::{Context as _, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read as _,
    os::unix::{ffi::OsStrExt as _, fs::PermissionsExt as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SourceSnapshot {
    pub(super) root: PathBuf,
    entries: BTreeMap<PathBuf, SourceEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceEntry {
    digest: String,
    modified: SystemTime,
}

impl SourceSnapshot {
    pub(super) fn capture(root: &Path) -> anyhow::Result<Self> {
        let output = Command::new("git")
            .current_dir(root)
            .args(["ls-files", "-z"])
            .stdin(Stdio::null())
            .output()
            .context("list checkout files for cache freshness")?;
        if !output.status.success() {
            bail!("cannot list checkout files for cache freshness");
        }
        let mut entries = BTreeMap::new();
        for bytes in output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let relative = PathBuf::from(std::ffi::OsStr::from_bytes(bytes));
            if let Some(entry) = fingerprint(root, &relative)? {
                entries.insert(relative, entry);
            }
        }
        Ok(Self {
            root: root.to_owned(),
            entries,
        })
    }

    pub(super) fn validate(&self) -> anyhow::Result<()> {
        if !self.root.is_absolute() {
            bail!("cached source root must be absolute");
        }
        for (path, entry) in &self.entries {
            files::validate_relative(path)?;
            if entry.digest.len() != 64
                || !entry
                    .digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                bail!("cached source digest is invalid");
            }
            entry.modified.duration_since(SystemTime::UNIX_EPOCH)?;
        }
        Ok(())
    }

    /// Inputs edited by the job cannot certify the outputs' original inputs.
    /// Omitting them makes a later checkout treat those paths as changed.
    pub(super) fn for_save(&self) -> anyhow::Result<Self> {
        let mut entries = BTreeMap::new();
        for (path, original) in &self.entries {
            if let Some(current) = fingerprint(&self.root, path)?
                && current.digest == original.digest
                && current.modified == original.modified
            {
                entries.insert(path.clone(), original.clone());
            }
        }
        Ok(Self {
            root: self.root.clone(),
            entries,
        })
    }

    pub(super) fn restore(&mut self, archives: &[RestoredArchive]) -> anyhow::Result<()> {
        let source_archives: Vec<_> = archives
            .iter()
            .filter(|archive| archive.sources.is_some())
            .collect();
        if source_archives.is_empty() {
            return Ok(());
        }
        if source_archives
            .iter()
            .any(|archive| archive.sources.as_ref().unwrap().root != self.root)
        {
            bail!("cached source metadata belongs to another workspace");
        }
        let newest_output = source_archives
            .iter()
            .map(|archive| archive.newest_output)
            .max()
            .unwrap();
        let changed_time = SystemTime::now().max(
            newest_output
                .checked_add(Duration::from_nanos(1))
                .context("cached output timestamp overflow")?,
        );
        for (path, current) in &mut self.entries {
            let mut unchanged_time = None;
            let unchanged_everywhere = source_archives.iter().all(|archive| {
                let saved = archive.sources.as_ref().unwrap();
                let Some(previous) = saved
                    .entries
                    .get(path)
                    .filter(|entry| entry.digest == current.digest)
                else {
                    return false;
                };
                unchanged_time = Some(
                    unchanged_time.map_or(previous.modified, |time: SystemTime| {
                        time.min(previous.modified)
                    }),
                );
                true
            });
            let modified = if unchanged_everywhere {
                unchanged_time.unwrap()
            } else {
                changed_time
            };
            files::set_modified(&self.root, path, modified)?;
            current.modified = modified;
        }
        Ok(())
    }
}

fn fingerprint(root: &Path, relative: &Path) -> anyhow::Result<Option<SourceEntry>> {
    let Some(metadata) = files::inspect(root, relative)? else {
        return Ok(None);
    };
    let path = root.join(relative);
    let mut digest = Sha256::new();
    if metadata.is_symlink() {
        digest.update(b"link");
        digest.update(fs::read_link(&path)?.as_os_str().as_bytes());
    } else if metadata.is_file() {
        digest.update(if metadata.permissions().mode() & 0o111 == 0 {
            b"file"
        } else {
            b"exec"
        });
        let mut file = files::open_file(&path)?;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            digest.update(&buffer[..read]);
        }
    } else {
        bail!("unsupported checkout entry: {}", path.display());
    }
    Ok(Some(SourceEntry {
        digest: hex::encode(digest.finalize()),
        modified: metadata.modified()?,
    }))
}
