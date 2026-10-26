use crate::git_repo::GitRepo;
use anyhow::{Context, bail, ensure};
use scope_domain::runs::source::{MAX_RUN_SOURCE_BUNDLE_BYTES, run_source_bundle_within_limit};
use std::{fs, process::Command};

pub(super) fn create_bundle(
    repo: &GitRepo,
    request_id: &str,
    oid: &str,
) -> anyhow::Result<Vec<u8>> {
    let prefix = format!("scope-run-upload-{request_id}-");
    let mut temp = tempfile::Builder::new();
    temp.prefix(&prefix);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temp.permissions(fs::Permissions::from_mode(0o700));
    }
    let temp = temp
        .tempdir()
        .context("create private run upload directory")?;
    let bundle_path = temp.path().join("source.bundle");
    let git_dir = temp.path().join("git");
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
    let size = fs::metadata(&bundle_path)
        .context("inspect exact Git bundle for Scope run")?
        .len();
    ensure_bundle_fits(size)?;
    fs::read(bundle_path).context("read exact Git bundle for Scope run")
}

fn ensure_bundle_fits(size: u64) -> anyhow::Result<()> {
    ensure!(
        run_source_bundle_within_limit(size),
        "Git bundle for this commit is {size} bytes, over Scope's {} MiB manual run upload limit. \
         Run a commit that is already the head of the Scope repository to use its stored source.",
        MAX_RUN_SOURCE_BUNDLE_BYTES / (1024 * 1024)
    );
    Ok(())
}

pub(super) fn random_request_id() -> anyhow::Result<String> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|error| anyhow::anyhow!("generate run request id: {error}"))?;
    Ok(hex::encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_size_check_admits_the_limit_and_rejects_one_byte_more() {
        let limit = MAX_RUN_SOURCE_BUNDLE_BYTES as u64;

        ensure_bundle_fits(limit).unwrap();
        let error = ensure_bundle_fits(limit + 1).unwrap_err().to_string();
        assert!(error.contains(&format!("{} bytes", limit + 1)), "{error}");
        assert!(error.contains("128 MiB manual run upload limit"), "{error}");
    }
}
