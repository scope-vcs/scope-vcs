use crate::{
    limits,
    paths::{extension, is_outside, is_resolution_config, resolve_lexically},
};
use rayon::prelude::*;
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

const SUPPORTED_SOURCE_EXTENSIONS: &[&str] =
    &[".cjs", ".cts", ".js", ".jsx", ".mjs", ".mts", ".ts", ".tsx"];

const UNSUPPORTED_SOURCE_EXTENSIONS: &[&str] = &[
    ".c", ".cc", ".clj", ".cljs", ".coffee", ".cpp", ".cs", ".dart", ".erl", ".ex", ".exs", ".fs",
    ".fsx", ".go", ".h", ".hpp", ".hrl", ".java", ".kt", ".kts", ".lua", ".php", ".pl", ".py",
    ".rb", ".rs", ".scala", ".svelte", ".swift", ".vue", ".astro",
];

const IGNORED_DIRECTORY_NAMES: &[&str] = &[".git", ".hg", ".svn", "node_modules"];

pub(crate) struct SnapshotError(pub(crate) String);

impl From<SnapshotError> for String {
    fn from(error: SnapshotError) -> Self {
        error.0
    }
}

pub(crate) struct Inventory {
    pub(crate) root: PathBuf,
    pub(crate) all_files: HashSet<String>,
    pub(crate) configs: Vec<String>,
    pub(crate) sources: Vec<String>,
    pub(crate) unsupported_files: Vec<String>,
}

impl Inventory {
    pub(crate) fn absolute(&self, path: &str) -> Result<PathBuf, SnapshotError> {
        absolute_snapshot_path(&self.root, path)
    }

    pub(crate) fn internal_package_names(&self) -> HashSet<String> {
        self.all_files
            .par_iter()
            .filter(|path| *path == "package.json" || path.ends_with("/package.json"))
            .filter_map(|path| {
                let text = fs::read(self.absolute(path).ok()?).ok()?;
                let manifest: serde_json::Value = serde_json::from_slice(&text).ok()?;
                let name = manifest.get("name")?.as_str()?;
                (!name.is_empty()).then(|| name.to_owned())
            })
            .collect()
    }
}

pub(crate) fn absolute_snapshot_path(
    root: &Path,
    path: impl AsRef<Path>,
) -> Result<PathBuf, SnapshotError> {
    let absolute = resolve_lexically(root, path);
    if is_outside(root, &absolute) {
        return Err(SnapshotError("snapshot path escapes its root".into()));
    }
    Ok(absolute)
}

pub(crate) fn inventory(requested_root: &Path) -> Result<Inventory, SnapshotError> {
    let root = fs::canonicalize(requested_root)
        .map_err(|error| SnapshotError(format!("cannot read snapshot directory: {error}")))?;
    if !root.is_dir() {
        return Err(SnapshotError("snapshot path is not a directory".into()));
    }

    let mut inventory = Inventory {
        root,
        all_files: HashSet::new(),
        configs: Vec::new(),
        sources: Vec::new(),
        unsupported_files: Vec::new(),
    };
    let mut total_input_bytes = 0_u64;
    let mut directories = vec![(inventory.root.clone(), String::new())];
    while let Some((absolute, parent)) = directories.pop() {
        let mut entries = fs::read_dir(&absolute)
            .and_then(|entries| entries.collect::<Result<Vec<_>, _>>())
            .map_err(inventory_error)?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if parent.is_empty() {
                name.clone()
            } else {
                format!("{parent}/{name}")
            };
            if path.len() > limits::MAX_PATH_BYTES {
                return Err(SnapshotError(format!(
                    "path length limit exceeded ({} bytes)",
                    limits::MAX_PATH_BYTES
                )));
            }
            let file_type = entry.file_type().map_err(inventory_error)?;
            if file_type.is_symlink() {
                return Err(SnapshotError(format!(
                    "symbolic links are not analyzed: {path}"
                )));
            }
            if file_type.is_dir() {
                if !IGNORED_DIRECTORY_NAMES.contains(&name.as_str()) {
                    directories.push((entry.path(), path));
                }
                continue;
            }
            if !file_type.is_file() {
                continue;
            }

            inventory.all_files.insert(path.clone());
            if inventory.all_files.len() > limits::MAX_FILES {
                return Err(SnapshotError(format!(
                    "file limit exceeded ({})",
                    limits::MAX_FILES
                )));
            }

            let extension = extension(&name);
            let supported = SUPPORTED_SOURCE_EXTENSIONS.contains(&extension.as_str());
            if is_resolution_config(&path) {
                inventory.configs.push(path.clone());
            }
            if supported || extension == ".json" || extension == ".jsonc" {
                let size = entry.metadata().map_err(inventory_error)?.len();
                if size > limits::MAX_SOURCE_BYTES {
                    return Err(SnapshotError(format!(
                        "analyzer input file size limit exceeded ({} bytes): {path}",
                        limits::MAX_SOURCE_BYTES
                    )));
                }
                total_input_bytes += size;
                if total_input_bytes > limits::MAX_TOTAL_SOURCE_BYTES {
                    return Err(SnapshotError(format!(
                        "total analyzer input size limit exceeded ({} bytes)",
                        limits::MAX_TOTAL_SOURCE_BYTES
                    )));
                }
            }

            if supported {
                inventory.sources.push(path);
            } else if UNSUPPORTED_SOURCE_EXTENSIONS.contains(&extension.as_str()) {
                inventory.unsupported_files.push(path);
                if inventory.unsupported_files.len() > limits::MAX_UNSUPPORTED_SOURCES {
                    return Err(SnapshotError(format!(
                        "unsupported source file limit exceeded ({})",
                        limits::MAX_UNSUPPORTED_SOURCES
                    )));
                }
            }
        }
    }

    inventory.configs.sort();
    inventory.sources.sort();
    inventory.unsupported_files.sort();
    Ok(inventory)
}

fn inventory_error(error: std::io::Error) -> SnapshotError {
    SnapshotError(format!("cannot inventory snapshot: {error}"))
}
