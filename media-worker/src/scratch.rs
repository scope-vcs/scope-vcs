use std::{
    fs, io,
    path::{Path, PathBuf},
};

const SCRATCH_PREFIX: &str = "job-";
const MAX_STARTUP_ENTRIES: usize = 1_024;

pub struct ScratchSpace {
    root: PathBuf,
}

impl ScratchSpace {
    pub fn prepare(root: PathBuf) -> anyhow::Result<Self> {
        reject_unsafe_root(&root)?;
        fs::create_dir_all(&root)?;
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            anyhow::bail!(
                "media scratch root must be a real directory: {}",
                root.display()
            );
        }
        let space = Self { root };
        space.clean_after_restart()?;
        Ok(space)
    }

    pub fn job(&self) -> anyhow::Result<tempfile::TempDir> {
        Ok(tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(&self.root)?)
    }

    fn clean_after_restart(&self) -> anyhow::Result<()> {
        let mut seen = 0_usize;
        for entry in fs::read_dir(&self.root)? {
            seen += 1;
            if seen > MAX_STARTUP_ENTRIES {
                anyhow::bail!(
                    "media scratch contains more than {MAX_STARTUP_ENTRIES} entries; refusing an unbounded startup cleanup"
                );
            }
            let entry = entry?;
            let name = entry.file_name();
            if !name.to_string_lossy().starts_with(SCRATCH_PREFIX) {
                anyhow::bail!(
                    "media scratch contains an unexpected entry: {}",
                    entry.path().display()
                );
            }
            remove_entry(&entry.path())?;
        }
        Ok(())
    }
}

fn reject_unsafe_root(root: &Path) -> anyhow::Result<()> {
    if !root.is_absolute() {
        anyhow::bail!("media scratch root must be absolute");
    }
    if root == Path::new("/") || root == Path::new("/tmp") {
        anyhow::bail!("media scratch root is too broad: {}", root.display());
    }
    Ok(())
}

fn remove_entry(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_removes_only_owned_job_directories() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("media");
        fs::create_dir_all(root.join("job-abandoned")).unwrap();
        fs::write(root.join("job-abandoned/source"), b"private").unwrap();

        let scratch = ScratchSpace::prepare(root.clone()).unwrap();
        assert!(fs::read_dir(&root).unwrap().next().is_none());
        let job = scratch.job().unwrap();
        assert!(job.path().starts_with(root));
    }

    #[test]
    fn startup_refuses_foreign_entries() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("media");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("do-not-delete"), b"kept").unwrap();

        assert!(ScratchSpace::prepare(root).is_err());
    }
}
