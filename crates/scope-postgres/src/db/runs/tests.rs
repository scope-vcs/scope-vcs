use super::*;
use crate::{
    db::{
        MetadataStore, acquire_aggregate_lock,
        generated_ids::test_generated_id,
        locks::wait_for_transaction_waiter,
        test_support::fixtures::{repository, source_blob, store_with_repositories, user},
    },
    error::PostgresErrorKind,
};
use scope_domain::{
    policy::Visibility,
    repository::collaboration::{RepositoryMember, RepositoryMemberPermissions},
    runs::{
        job::RunJobState,
        run::RunState,
        source::{RunSource, RunTrigger},
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use std::time::Duration;

const REPO: &str = "owner/repo";
const OWNER: &str = "user-owner";
const MEMBER: &str = "user-member";
const RUN: &str = "run-control";

#[derive(Clone, Copy, Debug)]
enum Command {
    Cancel,
    Retry,
}

impl Command {
    async fn execute(
        self,
        store: &MetadataStore,
        actor: &str,
        repository: &str,
    ) -> Result<Run, PostgresError> {
        match self {
            Self::Cancel => {
                store
                    .runs()
                    .request_run_cancellation(actor, repository, RUN, 20)
                    .await
            }
            Self::Retry => store.runs().retry_run(actor, repository, RUN, 20).await,
        }
    }

    fn expected_state(self) -> RunState {
        match self {
            Self::Cancel => RunState::Canceled,
            Self::Retry => RunState::Queued,
        }
    }
}

async fn fixture(command: Command) -> MetadataStore {
    let owner = user(OWNER, "owner");
    let mut repo = repository(&owner, "repo", Visibility::Public);
    repo.members.push(RepositoryMember {
        repo_id: REPO.into(),
        user_id: MEMBER.into(),
        permissions: RepositoryMemberPermissions {
            can_push: false,
            can_change_file_visibility: false,
        },
        created_at_unix: 1,
        updated_at_unix: 1,
    });
    let store = store_with_repositories([repo, repository(&owner, "other", Visibility::Public)]);
    let revision = scope_run_config::parse_workflow(
        "/.scope/runs/test.yml",
        br#"
name: Test
on:
  manual: true
container:
  image: alpine@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
timeout: 5m
jobs:
  checks:
    steps:
      - name: Test
        run: echo ok
"#,
    )
    .unwrap()
    .into_revision(REPO)
    .unwrap();
    let run = Run::new(
        RUN,
        RUN,
        revision.workflow().clone(),
        revision.digest(),
        RunTrigger::Manual,
        Some(OWNER.into()),
        RunSource::ephemeral_git_bundle(source_blob(&"c".repeat(40), &"b".repeat(64), 12)).unwrap(),
        10,
    )
    .unwrap();
    store.runs().enqueue_run(run, revision).await.unwrap();
    if matches!(command, Command::Retry) {
        store
            .runs()
            .request_run_cancellation(OWNER, REPO, RUN, 11)
            .await
            .unwrap();
    }
    store
}

async fn backend_pid(tx: &DatabaseTransaction) -> i32 {
    tx.query_one(Statement::from_string(
        DatabaseBackend::Postgres,
        "SELECT pg_backend_pid() AS pid",
    ))
    .await
    .unwrap()
    .unwrap()
    .try_get("", "pid")
    .unwrap()
}

async fn assert_success(store: &MetadataStore, command: Command, run: Run) {
    assert_eq!(run.state, command.expected_state());
    assert_eq!(store.runs().run(RUN).await.unwrap().unwrap(), run);
    let jobs = store.runs().run_jobs(RUN).await.unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].state,
        match command {
            Command::Cancel => RunJobState::Canceled,
            Command::Retry => RunJobState::Queued,
        }
    );
}

#[tokio::test]
async fn run_control_preserves_owner_and_member_access_and_rejects_outsiders() {
    for command in [Command::Cancel, Command::Retry] {
        for actor in [OWNER, MEMBER] {
            let store = fixture(command).await;
            let run = command.execute(&store, actor, REPO).await.unwrap();
            assert_success(&store, command, run).await;
        }
        let store = fixture(command).await;
        let before = store.runs().run(RUN).await.unwrap();
        let jobs = store.runs().run_jobs(RUN).await.unwrap();
        assert_eq!(
            command
                .execute(&store, "outsider", REPO)
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::PermissionDenied
        );
        assert_eq!(
            command
                .execute(&store, OWNER, "owner/other")
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::NotFound
        );
        assert_eq!(store.runs().run(RUN).await.unwrap(), before);
        assert_eq!(store.runs().run_jobs(RUN).await.unwrap(), jobs);
    }
}

#[tokio::test]
async fn run_control_waits_for_revocation_and_rejects_without_mutating() {
    for command in [Command::Cancel, Command::Retry] {
        let store = fixture(command).await;
        let before = store.runs().run(RUN).await.unwrap();
        let jobs = store.runs().run_jobs(RUN).await.unwrap();
        let revocation = store.db.begin().await.unwrap();
        acquire_aggregate_lock(&revocation, "repository", REPO)
            .await
            .unwrap();
        let pid = backend_pid(&revocation).await;
        entities::repository_member::Entity::delete_by_id((REPO.to_string(), MEMBER.to_string()))
            .exec(&revocation)
            .await
            .unwrap();
        let writing_store = store.clone();
        let writing =
            tokio::spawn(async move { command.execute(&writing_store, MEMBER, REPO).await });
        wait_for_transaction_waiter(&store, pid).await;
        revocation.commit().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(60), writing)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            result.unwrap_err().kind,
            PostgresErrorKind::PermissionDenied
        );
        assert_eq!(store.runs().run(RUN).await.unwrap(), before);
        assert_eq!(store.runs().run_jobs(RUN).await.unwrap(), jobs);
    }
}

#[tokio::test]
async fn run_control_that_wins_repository_lock_completes_before_real_revocation() {
    for command in [Command::Cancel, Command::Retry] {
        let store = fixture(command).await;
        // Hold jobs so the command pauses after acquiring its repository guard.
        let job_guard = store.db.begin().await.unwrap();
        locked_jobs(&job_guard, RUN).await.unwrap();
        let guard_pid = backend_pid(&job_guard).await;
        let writing_store = store.clone();
        let writing =
            tokio::spawn(async move { command.execute(&writing_store, MEMBER, REPO).await });
        let command_pid = wait_for_transaction_waiter(&store, guard_pid).await;
        let revoking_store = store.clone();
        let revoking = tokio::spawn(async move {
            revoking_store
                .repositories()
                .remove_repository_member("owner", "repo", OWNER, MEMBER, 21, &test_generated_id)
                .await
        });
        wait_for_transaction_waiter(&store, command_pid).await;
        job_guard.commit().await.unwrap();
        let run = tokio::time::timeout(Duration::from_secs(60), writing)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let removed = tokio::time::timeout(Duration::from_secs(60), revoking)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(removed.value.user_id, MEMBER);
        assert_success(&store, command, run).await;
        assert_eq!(
            command
                .execute(&store, MEMBER, REPO)
                .await
                .unwrap_err()
                .kind,
            PostgresErrorKind::PermissionDenied
        );
    }
}

#[tokio::test]
async fn authorized_retry_still_rejects_active_runs_without_changing_jobs() {
    let store = fixture(Command::Cancel).await;
    let before = store.runs().run(RUN).await.unwrap();
    let jobs = store.runs().run_jobs(RUN).await.unwrap();
    assert_eq!(
        Command::Retry
            .execute(&store, MEMBER, REPO)
            .await
            .unwrap_err()
            .kind,
        PostgresErrorKind::Conflict
    );
    assert_eq!(store.runs().run(RUN).await.unwrap(), before);
    assert_eq!(store.runs().run_jobs(RUN).await.unwrap(), jobs);
}
