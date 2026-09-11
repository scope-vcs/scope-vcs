use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    env,
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

const MAX_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;
const MAX_UPLOAD_RECEIPTS: usize = 256;
const MAX_PENDING_MUTATIONS: usize = 64;
const JOURNAL_KIND: &str = "scope.request-attachment-receipts";
const JOURNAL_VERSION: u8 = 1;

#[derive(Deserialize, Serialize)]
struct ReceiptJournal {
    kind: String,
    version: u8,
    uploads: Vec<UploadReceipt>,
    pending_mutations: Vec<PendingMutationReceipt>,
}

#[derive(Deserialize, Serialize)]
struct UploadReceipt {
    key: String,
    operation_id: String,
    updated_at_unix: u64,
}

#[derive(Deserialize, Serialize)]
struct PendingMutationReceipt {
    key: String,
    client_id: String,
    updated_at_unix: u64,
}

pub(crate) struct PendingMutation {
    key: String,
    pub(crate) client_id: String,
}

impl Default for ReceiptJournal {
    fn default() -> Self {
        Self {
            kind: JOURNAL_KIND.to_string(),
            version: JOURNAL_VERSION,
            uploads: Vec::new(),
            pending_mutations: Vec::new(),
        }
    }
}

pub(super) fn upload_operations(keys: &[&str]) -> anyhow::Result<Vec<String>> {
    let path = journal_path()?;
    let _lock = lock_journal(&path)?;
    let mut journal = load_journal(&path)?;
    let now = unix_now()?;
    let operations = keys
        .iter()
        .map(|key| journal.upload_operation_id(key, now))
        .collect::<anyhow::Result<Vec<_>>>()?;
    save_journal(&path, &mut journal)?;
    Ok(operations)
}

pub(super) fn rotate_upload_operation(
    key: &str,
    expired_operation_id: &str,
) -> anyhow::Result<String> {
    let path = journal_path()?;
    let _lock = lock_journal(&path)?;
    let mut journal = load_journal(&path)?;
    journal
        .uploads
        .retain(|receipt| receipt.key != key || receipt.operation_id != expired_operation_id);
    let operation_id = journal.upload_operation_id(key, unix_now()?)?;
    save_journal(&path, &mut journal)?;
    Ok(operation_id)
}

fn lock_journal(path: &Path) -> anyhow::Result<File> {
    ensure_journal_directory(
        path.parent()
            .context("attachment receipt journal has no parent")?,
    )?;
    let lock_path = path.with_extension("lock");
    reject_symlink(&lock_path, "Scope attachment receipt lock")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let lock = options
        .open(lock_path)
        .context("open Scope attachment receipt lock")?;
    lock.lock()
        .context("lock Scope attachment receipt journal")?;
    Ok(lock)
}

pub(crate) fn begin_mutation(
    api_url: &str,
    scope_fields: &[&str],
    id_prefix: &str,
) -> anyhow::Result<PendingMutation> {
    let mut key_fields = Vec::with_capacity(scope_fields.len() + 1);
    key_fields.push(api_url);
    key_fields.extend_from_slice(scope_fields);
    let key = fingerprint(&key_fields);
    let path = journal_path()?;
    let _lock = lock_journal(&path)?;
    let mut journal = load_journal(&path)?;
    let now = unix_now()?;
    let client_id = if let Some(receipt) = journal
        .pending_mutations
        .iter_mut()
        .find(|receipt| receipt.key == key)
    {
        receipt.updated_at_unix = now;
        receipt.client_id.clone()
    } else {
        let client_id = fresh_id(id_prefix)?;
        journal.pending_mutations.insert(
            0,
            PendingMutationReceipt {
                key: key.clone(),
                client_id: client_id.clone(),
                updated_at_unix: now,
            },
        );
        client_id
    };
    save_journal(&path, &mut journal)?;
    Ok(PendingMutation { key, client_id })
}

pub(crate) fn complete_uploads(receipt_keys: &[String]) -> anyhow::Result<()> {
    if receipt_keys.is_empty() {
        return Ok(());
    }
    let path = journal_path()?;
    let _lock = lock_journal(&path)?;
    let mut journal = load_journal(&path)?;
    journal
        .uploads
        .retain(|receipt| !receipt_keys.contains(&receipt.key));
    save_journal(&path, &mut journal)
}

pub(crate) fn complete_mutation(
    mutation: &PendingMutation,
    receipt_keys: &[String],
) -> anyhow::Result<()> {
    let path = journal_path()?;
    let _lock = lock_journal(&path)?;
    let mut journal = load_journal(&path)?;
    journal
        .uploads
        .retain(|receipt| !receipt_keys.contains(&receipt.key));
    journal
        .pending_mutations
        .retain(|receipt| receipt.key != mutation.key);
    save_journal(&path, &mut journal)
}

pub(super) fn fingerprint(fields: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for field in fields {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field.as_bytes());
    }
    hex::encode(hasher.finalize())
}

impl ReceiptJournal {
    fn upload_operation_id(&mut self, key: &str, now: u64) -> anyhow::Result<String> {
        if let Some(receipt) = self.uploads.iter_mut().find(|receipt| receipt.key == key) {
            receipt.updated_at_unix = now;
            return Ok(receipt.operation_id.clone());
        }
        let operation_id = fresh_id("cli_attachment")?;
        self.uploads.insert(
            0,
            UploadReceipt {
                key: key.to_string(),
                operation_id: operation_id.clone(),
                updated_at_unix: now,
            },
        );
        Ok(operation_id)
    }

    fn prune(&mut self) {
        self.uploads
            .sort_by_key(|receipt| std::cmp::Reverse(receipt.updated_at_unix));
        self.uploads.truncate(MAX_UPLOAD_RECEIPTS);
        self.pending_mutations
            .sort_by_key(|receipt| std::cmp::Reverse(receipt.updated_at_unix));
        self.pending_mutations.truncate(MAX_PENDING_MUTATIONS);
    }
}

fn fresh_id(prefix: &str) -> anyhow::Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random)
        .map_err(|error| anyhow::anyhow!("generate attachment operation ID: {error}"))?;
    Ok(format!("{prefix}_{}", hex::encode(random)))
}

fn load_journal(path: &Path) -> anyhow::Result<ReceiptJournal> {
    reject_symlink(path, "Scope attachment receipt journal")?;
    match fs::metadata(path) {
        Ok(metadata) if metadata.len() > MAX_JOURNAL_BYTES => {
            bail!("Scope attachment receipt journal is unexpectedly large")
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ReceiptJournal::default());
        }
        Err(error) => return Err(error).context("inspect Scope attachment receipt journal"),
    }
    let bytes = fs::read(path).context("read Scope attachment receipt journal")?;
    let journal: ReceiptJournal =
        serde_json::from_slice(&bytes).context("parse Scope attachment receipt journal")?;
    if journal.kind != JOURNAL_KIND || journal.version != JOURNAL_VERSION {
        bail!("Scope attachment receipt journal has an unsupported format");
    }
    Ok(journal)
}

fn save_journal(path: &Path, journal: &mut ReceiptJournal) -> anyhow::Result<()> {
    journal.prune();
    let bytes =
        serde_json::to_vec(journal).context("serialize Scope attachment receipt journal")?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        bail!("Scope attachment receipt journal exceeded its size bound");
    }
    let parent = path
        .parent()
        .context("Scope attachment receipt journal path has no parent")?;
    ensure_journal_directory(parent)?;
    reject_symlink(path, "Scope attachment receipt journal")?;
    let temp_path = parent.join(format!(
        ".request-attachments.{}.{}.tmp",
        std::process::id(),
        unix_now()?
    ));
    let result = (|| -> anyhow::Result<()> {
        write_private_file(&temp_path, &bytes)?;
        replace_journal_file(&temp_path, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn replace_journal_file(temp_path: &Path, path: &Path) -> anyhow::Result<()> {
    // std::fs::rename replaces an existing file on Windows as well as Unix.
    // Never unlink the durable journal before the replacement succeeds.
    fs::rename(temp_path, path).context("replace Scope attachment receipt journal")
}

fn journal_path() -> anyhow::Result<PathBuf> {
    let base = non_empty_path(env::var_os("XDG_CONFIG_HOME"))
        .or_else(|| non_empty_path(env::var_os("HOME")).map(|path| path.join(".config")))
        .or_else(|| non_empty_path(env::var_os("USERPROFILE")).map(|path| path.join(".config")))
        .context("locate Scope attachment receipt journal; set XDG_CONFIG_HOME or HOME")?;
    Ok(base.join("scope").join("request-attachments.json"))
}

fn non_empty_path(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|path| !path.is_empty()).map(PathBuf::from)
}

fn ensure_journal_directory(path: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("Scope attachment receipt directory cannot be a symlink")
        }
        Ok(metadata) if !metadata.is_dir() => {
            bail!("Scope attachment receipt path must be a directory")
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path).context("create Scope attachment receipt directory")?;
        }
        Err(error) => return Err(error).context("inspect Scope attachment receipt directory"),
    }
    secure_directory(path)
}

fn reject_symlink(path: &Path, label: &str) -> anyhow::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!("{label} cannot be a symlink"),
        Ok(metadata) if !metadata.is_file() => bail!("{label} must be a regular file"),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("inspect {label}")),
    }
}

fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .context("create temporary Scope attachment receipt journal")?;
    file.write_all(bytes)
        .context("write Scope attachment receipt journal")?;
    file.sync_all()
        .context("sync Scope attachment receipt journal")
}

fn secure_directory(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)
            .context("secure Scope attachment receipt directory")?;
    }
    Ok(())
}

pub(super) fn unix_now() -> anyhow::Result<u64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_journal_preserves_other_retry_identities() {
        let dir = crate::test_support::TestDir::new("replace-journal");
        let path = dir.path().join("receipts.json");
        let mut journal = ReceiptJournal::default();
        let first = journal.upload_operation_id("first", 1).unwrap();
        journal.pending_mutations.push(PendingMutationReceipt {
            key: "pending".into(),
            client_id: "client_pending".into(),
            updated_at_unix: 1,
        });
        save_journal(&path, &mut journal).unwrap();
        journal.upload_operation_id("second", 2).unwrap();
        save_journal(&path, &mut journal).unwrap();
        let stored = load_journal(&path).unwrap();
        assert_eq!(stored.uploads.len(), 2);
        assert_eq!(
            stored
                .uploads
                .iter()
                .find(|r| r.key == "first")
                .unwrap()
                .operation_id,
            first
        );
        assert_eq!(stored.pending_mutations[0].client_id, "client_pending");
    }

    #[test]
    fn failed_replacement_keeps_the_previous_journal() {
        let dir = crate::test_support::TestDir::new("failed-journal-replacement");
        let path = dir.path().join("receipts.json");
        fs::write(&path, b"durable retry identities").unwrap();
        assert!(replace_journal_file(&dir.path().join("missing-temp"), &path).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"durable retry identities");
    }

    #[cfg(windows)]
    #[test]
    fn windows_replacement_sharing_failure_keeps_both_files() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = crate::test_support::TestDir::new("locked-journal-replacement");
        let path = dir.path().join("receipts.json");
        let temp = dir.path().join("receipts.tmp");
        fs::write(&path, b"old retry identities").unwrap();
        fs::write(&temp, b"new retry identities").unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&temp)
            .unwrap();
        assert!(replace_journal_file(&temp, &path).is_err());
        drop(held);
        assert_eq!(fs::read(path).unwrap(), b"old retry identities");
        assert_eq!(fs::read(temp).unwrap(), b"new retry identities");
    }

    #[test]
    fn receipt_child() {
        let Ok(index) = env::var("SCOPE_TEST_RECEIPT_CHILD") else {
            return;
        };
        let start = PathBuf::from(env::var_os("SCOPE_TEST_RECEIPT_START").unwrap());
        while !start.exists() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        for iteration in 0..4 {
            let key = format!("{index}-{iteration}");
            let mutation = begin_mutation("https://api.example", &[&key], "discussion").unwrap();
            assert_eq!(
                begin_mutation("https://api.example", &[&key], "discussion")
                    .unwrap()
                    .client_id,
                mutation.client_id
            );
            let operation = upload_operations(&[&key]).unwrap();
            assert_eq!(upload_operations(&[&key]).unwrap(), operation);
        }
    }

    #[test]
    fn concurrent_processes_preserve_every_receipt() {
        let directory = env::temp_dir().join(fresh_id("scope-receipt-concurrency").unwrap());
        fs::create_dir(&directory).unwrap();
        let start = directory.as_path().join("start");
        let mut children = (0..12)
            .map(|index| {
                std::process::Command::new(env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "request::attachments::journal::tests::receipt_child",
                    ])
                    .env("XDG_CONFIG_HOME", directory.as_path())
                    .env("SCOPE_TEST_RECEIPT_CHILD", index.to_string())
                    .env("SCOPE_TEST_RECEIPT_START", &start)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect::<Vec<_>>();
        fs::write(start, "start").unwrap();
        for child in &mut children {
            assert!(child.wait().unwrap().success());
        }
        let journal =
            load_journal(&directory.as_path().join("scope/request-attachments.json")).unwrap();
        assert_eq!(journal.uploads.len(), 48);
        assert_eq!(journal.pending_mutations.len(), 48);
        fs::remove_dir_all(directory).unwrap();
    }
}
