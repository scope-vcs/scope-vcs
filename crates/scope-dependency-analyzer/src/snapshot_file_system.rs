use crate::{
    paths::{normalize_lexically, repository_path},
    snapshot::Inventory,
};
use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};
use std::{
    collections::HashSet,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

const MODULE_EXTENSIONS: &[&str] = &[
    ".cts", ".mts", ".tsx", ".ts", ".cjs", ".mjs", ".jsx", ".js", ".json",
];

const FILE: FileMetadata = FileMetadata::new(true, false, false);
const DIRECTORY: FileMetadata = FileMetadata::new(false, true, false);

#[derive(Clone, Default)]
pub(crate) struct SnapshotFileSystem {
    root: Arc<PathBuf>,
    files: Arc<HashSet<String>>,
    directories: Arc<HashSet<String>>,
    suffixes: Arc<[String]>,
}

impl SnapshotFileSystem {
    pub(crate) fn new(inventory: &Inventory, suffixes: Vec<String>) -> Self {
        Self {
            root: Arc::new(inventory.root.clone()),
            files: inventory.all_files.clone(),
            directories: inventory.directories.clone(),
            suffixes: suffixes.into(),
        }
    }

    pub(crate) fn snapshot_file(&self, resolved: &Path) -> Option<String> {
        let located = self.suffixed(resolved);
        let (path, metadata) = self.entry(located.as_deref().unwrap_or(resolved))?;
        metadata.is_file().then_some(path)
    }

    fn entry(&self, path: &Path) -> Option<(String, FileMetadata)> {
        let path = repository_path(&self.root, &normalize_lexically(path))?;
        if self.files.contains(&path) {
            Some((path, FILE))
        } else if self.directories.contains(&path) {
            Some((path, DIRECTORY))
        } else {
            None
        }
    }

    fn absolute(&self, path: &str) -> PathBuf {
        if path.is_empty() {
            self.root.to_path_buf()
        } else {
            self.root.join(path)
        }
    }

    fn file_path(&self, path: &Path) -> io::Result<PathBuf> {
        match self.entry(path) {
            Some((path, metadata)) if metadata.is_file() => Ok(self.absolute(&path)),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
    }

    fn suffixed(&self, path: &Path) -> Option<PathBuf> {
        if self.suffixes.is_empty() {
            return None;
        }
        let (stem, extension) = module_file_parts(path)?;
        self.suffixes
            .iter()
            .map(|suffix| PathBuf::from(format!("{stem}{suffix}{extension}")))
            .find(|candidate| {
                self.entry(candidate)
                    .is_some_and(|(_, metadata)| metadata.is_file())
            })
    }

    fn suffixed_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        let metadata = if self.suffixes.is_empty() || module_file_parts(path).is_none() {
            self.entry(path).map(|(_, metadata)| metadata)
        } else if self.suffixed(path).is_some() {
            Some(FILE)
        } else {
            self.entry(path)
                .map(|(_, metadata)| metadata)
                .filter(|metadata| metadata.is_dir())
        };
        metadata.ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}

fn module_file_parts(path: &Path) -> Option<(&str, &str)> {
    let path = path.to_str()?;
    let name = path.rsplit(['/', '\\']).next()?;
    if name == "package.json" {
        return None;
    }
    let extension = MODULE_EXTENSIONS
        .iter()
        .find(|extension| path.ends_with(*extension))?;
    let stem = &path[..path.len() - extension.len()];
    let start = stem
        .strip_suffix(".d")
        .map_or(path.len() - extension.len(), str::len);
    Some((&path[..start], &path[start..]))
}

impl FileSystem for SnapshotFileSystem {
    fn new() -> Self {
        Self::default()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        FileSystemOs.read(&self.file_path(path)?)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        FileSystemOs.read_to_string(&self.file_path(path)?)
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.suffixed_metadata(path)
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.suffixed_metadata(path)
    }

    fn read_link(&self, _path: &Path) -> Result<PathBuf, ResolveError> {
        Err(io::Error::from(io::ErrorKind::InvalidInput).into())
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        self.entry(path)
            .map(|(path, _)| self.absolute(&path))
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
}
