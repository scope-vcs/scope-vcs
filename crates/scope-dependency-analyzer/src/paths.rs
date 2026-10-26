use std::path::{Component, Path, PathBuf};

pub(crate) fn resolve_lexically(base: &Path, relative: impl AsRef<Path>) -> PathBuf {
    let mut resolved = PathBuf::new();
    for component in base.join(relative).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                resolved.pop();
            }
            other => resolved.push(other),
        }
    }
    resolved
}

pub(crate) fn is_outside(root: &Path, candidate: &Path) -> bool {
    !candidate.starts_with(root)
}

pub(crate) fn parent_directory(path: &str) -> &str {
    path.rfind('/').map_or(".", |index| &path[..index])
}

pub(crate) fn normalize_relative(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." if segments.last().is_some_and(|last| *last != "..") => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        ".".into()
    } else {
        segments.join("/")
    }
}

pub(crate) fn join_relative(directory: &str, path: &str) -> String {
    if directory == "." {
        normalize_relative(path)
    } else {
        normalize_relative(&format!("{directory}/{path}"))
    }
}

pub(crate) fn repository_path(root: &Path, absolute: &Path) -> Option<String> {
    let relative = absolute.strip_prefix(root).ok()?;
    let segments = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>();
    Some(segments.join("/"))
}

pub(crate) fn extension(name: &str) -> String {
    match name.rfind('.') {
        Some(index) if index > 0 => name[index..].to_ascii_lowercase(),
        _ => String::new(),
    }
}

pub(crate) fn file_name(path: &str) -> &str {
    path.rfind('/').map_or(path, |index| &path[index + 1..])
}

pub(crate) fn is_resolution_config(path: &str) -> bool {
    let name = file_name(path);
    let Some(rest) = name
        .strip_prefix("tsconfig")
        .or_else(|| name.strip_prefix("jsconfig"))
        .and_then(|rest| rest.strip_suffix(".json"))
    else {
        return false;
    };
    rest.is_empty()
        || rest.strip_prefix('.').is_some_and(|segments| {
            segments
                .split('.')
                .all(|segment| !segment.is_empty() && !segment.contains('/'))
        })
}
