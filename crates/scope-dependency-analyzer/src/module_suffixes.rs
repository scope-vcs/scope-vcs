use oxc_resolver::{FileMetadata, FileSystem, FileSystemOs, ResolveError};
use std::{
    borrow::Cow,
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

const MODULE_EXTENSIONS: &[&str] = &[
    ".cts", ".mts", ".tsx", ".ts", ".cjs", ".mjs", ".jsx", ".js", ".json",
];

#[derive(Clone, Default)]
pub(crate) struct SuffixFileSystem {
    suffixes: Arc<[String]>,
}

impl SuffixFileSystem {
    pub(crate) fn new(suffixes: Vec<String>) -> Self {
        Self {
            suffixes: suffixes.into(),
        }
    }

    pub(crate) fn locate<'a>(&self, path: &'a Path) -> Cow<'a, Path> {
        if self.suffixes.is_empty() {
            return Cow::Borrowed(path);
        }
        let Some((stem, extension)) = module_file_parts(path) else {
            return Cow::Borrowed(path);
        };
        self.suffixes
            .iter()
            .map(|suffix| PathBuf::from(format!("{stem}{suffix}{extension}")))
            .find(|candidate| {
                FileSystemOs
                    .symlink_metadata(candidate)
                    .is_ok_and(|metadata| metadata.is_file())
            })
            .map_or(Cow::Borrowed(path), Cow::Owned)
    }

    fn suffixed_metadata(
        &self,
        path: &Path,
        metadata: impl Fn(&Path) -> io::Result<FileMetadata>,
    ) -> io::Result<FileMetadata> {
        if self.suffixes.is_empty() || module_file_parts(path).is_none() {
            return metadata(path);
        }
        let located = self.locate(path);
        if let Cow::Owned(located) = located {
            return metadata(&located);
        }
        match metadata(path) {
            Ok(found) if found.is_dir() => Ok(found),
            _ => Err(io::ErrorKind::NotFound.into()),
        }
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

impl FileSystem for SuffixFileSystem {
    fn new() -> Self {
        Self::default()
    }

    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        FileSystemOs.read(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        FileSystemOs.read_to_string(path)
    }

    fn metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.suffixed_metadata(path, |path| FileSystemOs.metadata(path))
    }

    fn symlink_metadata(&self, path: &Path) -> io::Result<FileMetadata> {
        self.suffixed_metadata(path, |path| FileSystemOs.symlink_metadata(path))
    }

    fn read_link(&self, path: &Path) -> Result<PathBuf, ResolveError> {
        FileSystemOs.read_link(path)
    }

    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        FileSystemOs.canonicalize(path)
    }
}
