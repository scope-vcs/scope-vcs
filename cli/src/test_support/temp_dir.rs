use std::path::Path;

pub(crate) struct TempDir(tempfile::TempDir);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        Self(
            tempfile::Builder::new()
                .prefix(&format!("scope-cli-{label}-"))
                .tempdir()
                .unwrap(),
        )
    }

    pub(crate) fn path(&self) -> &Path {
        self.0.path()
    }
}
