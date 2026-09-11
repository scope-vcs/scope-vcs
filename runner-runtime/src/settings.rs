use anyhow::Context as _;
use std::{env, path::PathBuf};

pub struct RuntimeSettings {
    pub api_url: String,
    pub attempt_id: String,
    pub bootstrap_token: String,
    pub attempt_deadline_unix: u64,
    pub work_root: PathBuf,
}

impl RuntimeSettings {
    pub fn from_env() -> anyhow::Result<Self> {
        let api_url = scope_service_config::ServiceEndpoint::parse(&required("SCOPE_API_URL")?)
            .context("SCOPE_API_URL")?
            .as_str()
            .to_string();
        let attempt_deadline_unix = required("SCOPE_ATTEMPT_DEADLINE_UNIX")?
            .parse::<u64>()
            .context("SCOPE_ATTEMPT_DEADLINE_UNIX must be an unsigned Unix timestamp")?;
        Ok(Self {
            api_url,
            attempt_id: required("SCOPE_ATTEMPT_ID")?,
            bootstrap_token: required("SCOPE_BOOTSTRAP_TOKEN")?,
            attempt_deadline_unix,
            work_root: env::var_os("SCOPE_WORK_ROOT")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/scope/work")),
        })
    }

    pub fn prepare_work_directory(&self) -> anyhow::Result<PathBuf> {
        // Each task has its own filesystem. A stable path lets build caches reuse
        // fingerprints that contain absolute source paths across attempts.
        let root = self.work_root.join("job");
        std::fs::create_dir_all(&self.work_root).context("create runtime work root")?;
        std::fs::create_dir(&root).context("create fresh attempt work directory")?;
        std::fs::canonicalize(root).context("resolve attempt work directory")
    }
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
        let mut settings = RuntimeSettings {
            api_url: String::new(),
            attempt_id: "first".to_string(),
            bootstrap_token: String::new(),
            attempt_deadline_unix: 0,
            work_root: root.path().to_owned(),
        };
        let first = settings.prepare_work_directory().unwrap();
        std::fs::write(first.join("sentinel"), "previous attempt").unwrap();
        settings.attempt_id = "second".to_string();
        assert!(settings.prepare_work_directory().is_err());
        assert!(first.join("sentinel").exists());
        std::fs::remove_dir_all(&first).unwrap();
        assert_eq!(settings.prepare_work_directory().unwrap(), first);
    }
}
