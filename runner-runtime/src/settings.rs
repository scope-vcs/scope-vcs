use anyhow::{Context as _, bail};
use std::{
    env,
    path::{Path, PathBuf},
};

/// Each task has its own filesystem. A stable path lets build caches reuse
/// fingerprints that contain absolute source paths across attempts.
pub const WORK_ROOT: &str = "/scope/work";

pub struct RuntimeSettings {
    pub api_url: String,
    pub attempt_id: String,
    pub bootstrap_token: String,
    pub attempt_deadline_unix: u64,
}

impl RuntimeSettings {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_url = required("SCOPE_API_URL")?.trim_end_matches('/').to_string();
        if !(api_url.starts_with("https://") || api_url.starts_with("http://127.0.0.1")) {
            bail!("SCOPE_API_URL must use HTTPS outside local development");
        }
        let attempt_deadline_unix = required("SCOPE_ATTEMPT_DEADLINE_UNIX")?
            .parse::<u64>()
            .context("SCOPE_ATTEMPT_DEADLINE_UNIX must be an unsigned Unix timestamp")?;
        Ok(Self {
            api_url,
            attempt_id: required("SCOPE_ATTEMPT_ID")?,
            bootstrap_token: required("SCOPE_BOOTSTRAP_TOKEN")?,
            attempt_deadline_unix,
        })
    }
}

/// Creates the attempt's work directory under `work_root`, refusing to reuse
/// one left behind by an earlier attempt.
pub fn prepare_work_directory(work_root: &Path) -> anyhow::Result<PathBuf> {
    let root = work_root.join("job");
    std::fs::create_dir_all(work_root).context("create runtime work root")?;
    std::fs::create_dir(&root).context("create fresh attempt work directory")?;
    std::fs::canonicalize(root).context("resolve attempt work directory")
}

fn required(name: &str) -> anyhow::Result<String> {
    env::var_os(name)
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("{name} is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_attempts_use_the_same_path_and_refuse_existing_work() {
        let root = tempfile::tempdir().unwrap();
        let first = prepare_work_directory(root.path()).unwrap();
        std::fs::write(first.join("sentinel"), "previous attempt").unwrap();
        assert!(prepare_work_directory(root.path()).is_err());
        assert!(first.join("sentinel").exists());
        std::fs::remove_dir_all(&first).unwrap();
        assert_eq!(prepare_work_directory(root.path()).unwrap(), first);
    }
}
