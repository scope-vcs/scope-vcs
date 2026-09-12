use crate::env_guard::{require_exact, required};
use crate::storage_runtime::{StorageRuntime, StorageSource};
use crate::{
    auth::cli::CliAuthService,
    config::{database_url_from_env, non_empty_env},
    demo_seed::{DevSeedUser, catalog, seed_request_discussion_gallery, seed_user_account},
    persistence::unix_now,
};
use scope_postgres::db::MetadataStore;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const OPT_IN_ENV: &str = "SCOPE_ALLOW_STAGING_SMOKE_SEED";
const EXPECTED_PROJECT_ID_ENV: &str = "SCOPE_SMOKE_SEED_PROJECT_ID";
const EXPECTED_ENVIRONMENT_ID_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_ID";
const EXPECTED_ENVIRONMENT_NAME_ENV: &str = "SCOPE_SMOKE_SEED_ENVIRONMENT_NAME";
const PRODUCTION_ENVIRONMENT_ID_ENV: &str = "SCOPE_PRODUCTION_ENVIRONMENT_ID";
const SEED_USER_EMAIL_ENV: &str = "SCOPE_SMOKE_SEED_USER_EMAIL";
const SEED_USER_HANDLE_ENV: &str = "SCOPE_SMOKE_SEED_USER_HANDLE";
const EXCHANGE_TOKEN_PATH_ENV: &str = "SCOPE_SMOKE_SEED_EXCHANGE_TOKEN_PATH";

#[derive(Debug, PartialEq, Eq)]
struct Target {
    seed_user: DevSeedUser,
    exchange_token_path: PathBuf,
}

#[derive(Default)]
struct Snapshot {
    opt_in: Option<String>,
    expected_project_id: Option<String>,
    expected_environment_id: Option<String>,
    expected_environment_name: Option<String>,
    production_environment_id: Option<String>,
    actual_project_id: Option<String>,
    actual_environment_id: Option<String>,
    actual_environment_name: Option<String>,
    seed_user_email: Option<String>,
    seed_user_handle: Option<String>,
    exchange_token_path: Option<String>,
}

impl Snapshot {
    fn from_env() -> Self {
        Self {
            opt_in: non_empty_env(OPT_IN_ENV),
            expected_project_id: non_empty_env(EXPECTED_PROJECT_ID_ENV),
            expected_environment_id: non_empty_env(EXPECTED_ENVIRONMENT_ID_ENV),
            expected_environment_name: non_empty_env(EXPECTED_ENVIRONMENT_NAME_ENV),
            production_environment_id: non_empty_env(PRODUCTION_ENVIRONMENT_ID_ENV),
            actual_project_id: non_empty_env("RAILWAY_PROJECT_ID"),
            actual_environment_id: non_empty_env("RAILWAY_ENVIRONMENT_ID"),
            actual_environment_name: non_empty_env("RAILWAY_ENVIRONMENT_NAME"),
            seed_user_email: non_empty_env(SEED_USER_EMAIL_ENV),
            seed_user_handle: non_empty_env(SEED_USER_HANDLE_ENV),
            exchange_token_path: non_empty_env(EXCHANGE_TOKEN_PATH_ENV),
        }
    }
}

pub async fn run(grant_only: bool) -> anyhow::Result<()> {
    run_with_snapshot(Snapshot::from_env(), grant_only, async {
        MetadataStore::connect(database_url_from_env()?).await
    })
    .await
}

async fn run_with_snapshot(
    snapshot: Snapshot,
    grant_only: bool,
    connect: impl std::future::Future<Output = anyhow::Result<MetadataStore>>,
) -> anyhow::Result<()> {
    let target = validate(&snapshot)?;
    let mut exchange_token_file = create_exchange_token_file(&target.exchange_token_path)?;
    let seed_user = seed_user_account(target.seed_user.clone());
    let metadata = connect.await?;
    // Imported releases keep the existing catalog while obtaining a fresh smoke login.
    if !grant_only {
        let storage = StorageRuntime::from_env(StorageSource::S3).await?;
        let fixture = catalog(
            storage.object_store.as_ref(),
            storage.git_segment_store.as_ref(),
            target.seed_user,
        )
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
        metadata
            .admin()
            .replace_catalog_for_seed(fixture)
            .await
            .map_err(|error| anyhow::anyhow!("replacing staging smoke catalog: {error}"))?;
        seed_request_discussion_gallery(&metadata)
            .await
            .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    }
    let now_unix = unix_now().map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    let grant = CliAuthService::new(metadata.auth())
        .create_staging_smoke_exchange_grant(&seed_user, now_unix)
        .await
        .map_err(|error| anyhow::anyhow!(error.into_operator_diagnostic()))?;
    write_exchange_token(&mut exchange_token_file, &grant.exchange_token)?;
    if grant_only {
        println!(r#"{{"exchangeGrantCreated":true}}"#);
    } else {
        println!(r#"{{"seeded":"dev/public-demo"}}"#);
    }
    Ok(())
}

fn validate(snapshot: &Snapshot) -> anyhow::Result<Target> {
    require_exact(OPT_IN_ENV, snapshot.opt_in.as_deref(), "1")?;
    let expected_project_id = required(EXPECTED_PROJECT_ID_ENV, &snapshot.expected_project_id)?;
    let expected_environment_id = required(
        EXPECTED_ENVIRONMENT_ID_ENV,
        &snapshot.expected_environment_id,
    )?;
    let expected_environment_name = required(
        EXPECTED_ENVIRONMENT_NAME_ENV,
        &snapshot.expected_environment_name,
    )?;
    let production_environment_id = required(
        PRODUCTION_ENVIRONMENT_ID_ENV,
        &snapshot.production_environment_id,
    )?;
    if expected_environment_id == production_environment_id {
        anyhow::bail!("staging smoke seed target matches the production environment");
    }
    require_exact(
        "RAILWAY_PROJECT_ID",
        snapshot.actual_project_id.as_deref(),
        expected_project_id,
    )?;
    require_exact(
        "RAILWAY_ENVIRONMENT_ID",
        snapshot.actual_environment_id.as_deref(),
        expected_environment_id,
    )?;
    require_exact(
        "RAILWAY_ENVIRONMENT_NAME",
        snapshot.actual_environment_name.as_deref(),
        expected_environment_name,
    )?;
    if snapshot.actual_environment_id.as_deref() == Some(production_environment_id) {
        anyhow::bail!("refusing to replace the production catalog");
    }

    let email = required(SEED_USER_EMAIL_ENV, &snapshot.seed_user_email)?;
    if !email.contains('@') {
        anyhow::bail!("{SEED_USER_EMAIL_ENV} must be an email address");
    }
    let handle = required(SEED_USER_HANDLE_ENV, &snapshot.seed_user_handle)?;
    if !handle
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        anyhow::bail!("{SEED_USER_HANDLE_ENV} must contain only letters, numbers, or hyphens");
    }
    let exchange_token_path = PathBuf::from(required(
        EXCHANGE_TOKEN_PATH_ENV,
        &snapshot.exchange_token_path,
    )?);
    if !exchange_token_path.is_absolute() {
        anyhow::bail!("{EXCHANGE_TOKEN_PATH_ENV} must be an absolute path");
    }
    if exchange_token_path.file_name().is_none() {
        anyhow::bail!("{EXCHANGE_TOKEN_PATH_ENV} must identify a file");
    }

    Ok(Target {
        seed_user: DevSeedUser {
            email: email.to_string(),
            handle: handle.to_string(),
        },
        exchange_token_path,
    })
}

fn create_exchange_token_file(path: &Path) -> anyhow::Result<File> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("{EXCHANGE_TOKEN_PATH_ENV} must have a parent directory"))?;
    let parent_metadata = std::fs::symlink_metadata(parent)
        .map_err(|error| anyhow::anyhow!("inspecting exchange token directory: {error}"))?;
    if !parent_metadata.file_type().is_dir() {
        anyhow::bail!("staging smoke exchange token parent must be a directory");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if parent_metadata.permissions().mode() & 0o077 != 0 {
            anyhow::bail!(
                "staging smoke exchange token directory must not be accessible by group or other users"
            );
        }
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(path)
        .map_err(|error| anyhow::anyhow!("creating staging smoke exchange token file: {error}"))?;
    Ok(file)
}

fn write_exchange_token(file: &mut File, token: &str) -> anyhow::Result<()> {
    file.write_all(token.as_bytes())
        .and_then(|()| file.write_all(b"\n"))
        .and_then(|()| file.sync_all())
        .map_err(|error| anyhow::anyhow!("writing staging smoke exchange token file: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = file
            .metadata()
            .map_err(|error| anyhow::anyhow!("reading exchange token file mode: {error}"))?
            .permissions()
            .mode()
            & 0o777;
        if mode != 0o600 {
            anyhow::bail!("staging smoke exchange token file must have mode 0600");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_snapshot() -> Snapshot {
        Snapshot {
            opt_in: Some("1".into()),
            expected_project_id: Some("project-staging".into()),
            expected_environment_id: Some("environment-staging".into()),
            expected_environment_name: Some("staging".into()),
            production_environment_id: Some("environment-production".into()),
            actual_project_id: Some("project-staging".into()),
            actual_environment_id: Some("environment-staging".into()),
            actual_environment_name: Some("staging".into()),
            seed_user_email: Some("smoke@example.test".into()),
            seed_user_handle: Some("dev".into()),
            exchange_token_path: Some("/tmp/scope-smoke/exchange-token".into()),
        }
    }

    #[tokio::test]
    async fn grant_only_preserves_existing_catalog_and_issues_redeemable_login() {
        use scope_domain::{account::UserAccount, policy::Visibility, repository::Repository};
        use scope_postgres::db::{CatalogFixture, TestDatabaseTarget};

        let target = TestDatabaseTarget::required().unwrap();
        let metadata = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let owner = UserAccount {
            id: "user_keep_catalog".into(),
            handle: "keep-owner".into(),
            email: "keep@example.test".into(),
            email_verified: true,
        };
        let repository = Repository::new(
            &owner,
            "keep-repo",
            Visibility::Private,
            "repoi_keep_catalog",
        )
        .unwrap();
        let mut fixture = CatalogFixture::default();
        fixture.users.insert(owner.id.clone(), owner.clone());
        let smoke_user = seed_user_account(DevSeedUser {
            email: "smoke@example.test".into(),
            handle: "dev".into(),
        });
        fixture.users.insert(smoke_user.id.clone(), smoke_user);
        fixture
            .repositories
            .insert(repository.record.id.clone(), repository.clone());
        metadata.admin().seed_catalog_for_tests(fixture).unwrap();
        let temp = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = temp.path().join("exchange-token");
        let mut snapshot = valid_snapshot();
        snapshot.exchange_token_path = Some(path.to_str().unwrap().into());
        run_with_snapshot(snapshot, true, std::future::ready(Ok(metadata.clone())))
            .await
            .unwrap();

        let retained = metadata
            .repositories()
            .repository("keep-owner", "keep-repo")
            .await
            .unwrap()
            .expect("grant-only must retain the existing repository");
        assert_eq!(
            serde_json::to_value(retained).unwrap(),
            serde_json::to_value(repository).unwrap()
        );
        assert_eq!(
            metadata
                .repositories()
                .repository_count_for_tests()
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            metadata.auth().user_for_tests(&owner.id).await.unwrap(),
            Some(owner)
        );
        let token = std::fs::read_to_string(path).unwrap();
        let session = CliAuthService::new(metadata.auth())
            .exchange_grant(token.trim(), unix_now().unwrap())
            .await
            .unwrap();
        assert_eq!(
            session.identity.email.as_deref(),
            Some("smoke@example.test")
        );
    }

    #[tokio::test]
    async fn grant_only_rejects_unreviewed_targets_before_side_effects() {
        for mutate in [
            |snapshot: &mut Snapshot| snapshot.opt_in = None,
            |snapshot: &mut Snapshot| snapshot.actual_project_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_name = Some("wrong".into()),
            |snapshot: &mut Snapshot| {
                snapshot.expected_environment_id = snapshot.production_environment_id.clone()
            },
        ] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("exchange-token");
            let mut snapshot = valid_snapshot();
            snapshot.exchange_token_path = Some(path.to_str().unwrap().into());
            mutate(&mut snapshot);
            let result = run_with_snapshot(snapshot, true, async {
                panic!("rejected target must not connect to the database")
            })
            .await;
            assert!(result.is_err());
            assert!(
                !path.exists(),
                "rejected target must not create a token file"
            );
        }
    }

    #[test]
    fn accepts_the_reviewed_staging_target() {
        assert_eq!(
            validate(&valid_snapshot()).unwrap(),
            Target {
                seed_user: DevSeedUser {
                    email: "smoke@example.test".into(),
                    handle: "dev".into(),
                },
                exchange_token_path: PathBuf::from("/tmp/scope-smoke/exchange-token"),
            }
        );
    }

    #[test]
    fn requires_explicit_opt_in() {
        let mut snapshot = valid_snapshot();
        snapshot.opt_in = None;
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains(OPT_IN_ENV)
        );
    }

    #[test]
    fn rejects_the_production_environment() {
        let mut snapshot = valid_snapshot();
        snapshot.actual_environment_id = snapshot.production_environment_id.clone();
        snapshot.expected_environment_id = snapshot.production_environment_id.clone();
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("production")
        );
    }

    #[test]
    fn rejects_a_different_project_or_environment() {
        for mutate in [
            |snapshot: &mut Snapshot| snapshot.actual_project_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_id = Some("wrong".into()),
            |snapshot: &mut Snapshot| snapshot.actual_environment_name = Some("wrong".into()),
        ] {
            let mut snapshot = valid_snapshot();
            mutate(&mut snapshot);
            assert!(validate(&snapshot).is_err());
        }
    }

    #[test]
    fn rejects_a_relative_exchange_token_path() {
        let mut snapshot = valid_snapshot();
        snapshot.exchange_token_path = Some("exchange-token".into());
        assert!(
            validate(&snapshot)
                .unwrap_err()
                .to_string()
                .contains("absolute")
        );
    }

    #[cfg(unix)]
    #[test]
    fn creates_the_exchange_token_file_once_with_private_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = temp.path().join("exchange-token");
        let mut file = create_exchange_token_file(&path).unwrap();
        write_exchange_token(&mut file, "scope_otc_test").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "scope_otc_test\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(create_exchange_token_file(&path).is_err());
    }
}
