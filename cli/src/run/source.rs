use crate::git_repo::GitRepo;
use anyhow::{Context, bail};
use std::{env, fs, path::PathBuf, process::Command};

pub(super) fn create_bundle(
    repo: &GitRepo,
    request_id: &str,
    oid: &str,
) -> anyhow::Result<Vec<u8>> {
    let temp = BundleTemp::new(request_id)?;
    let bundle_path = temp.path.join("source.bundle");
    // A private Git directory gives the captured commit a stable HEAD without
    // changing refs in the user's checkout, including during concurrent commits.
    let git_dir = temp.path.join("git");
    let init = Command::new("git")
        .args(["init", "--bare", "--quiet"])
        .arg(&git_dir)
        .output()?;
    anyhow::ensure!(init.status.success(), "initialize source bundle repository");
    let objects = Command::new("git")
        .current_dir(&repo.root)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ])
        .output()?;
    anyhow::ensure!(objects.status.success(), "locate source Git objects");
    fs::write(git_dir.join("objects/info/alternates"), &objects.stdout)?;
    fs::write(git_dir.join("HEAD"), format!("{oid}\n"))?;
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(&git_dir)
        .args(["bundle", "create"])
        .arg(&bundle_path)
        .arg("HEAD")
        .output()
        .context("create exact Git bundle for Scope run")?;
    if !output.status.success() {
        bail!(
            "create exact Git bundle for Scope run: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&bundle_path, fs::Permissions::from_mode(0o600))
            .context("secure exact Git bundle for Scope run")?;
    }
    fs::read(bundle_path).context("read exact Git bundle for Scope run")
}

pub(super) fn random_request_id() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("generate run request id: {error}"))?;
    Ok(hex::encode(bytes))
}

struct BundleTemp {
    path: PathBuf,
}

impl BundleTemp {
    fn new(request_id: &str) -> anyhow::Result<Self> {
        let path = env::temp_dir().join(format!("scope-run-upload-{request_id}"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new()
                .mode(0o700)
                .create(&path)
                .context("create private run upload directory")?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&path).context("create run upload directory")?;
        Ok(Self { path })
    }
}

impl Drop for BundleTemp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
