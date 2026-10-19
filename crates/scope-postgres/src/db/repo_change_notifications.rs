use super::RepositoryStore;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::mpsc;

pub(crate) const POSTGRES_REPO_CHANGE_CHANNEL: &str = "scope_repo_changes";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const QUEUED_NOTIFICATION_LIMIT: usize = 16_384;
const NOTIFICATIONS_PER_COMMIT: usize = 256;
type PayloadHandler = Arc<dyn Fn(String) + Send + Sync>;

impl RepositoryStore {
    pub fn start_repo_change_listener(
        &self,
        on_payload: impl Fn(String) + Send + Sync + 'static,
    ) -> anyhow::Result<()> {
        let Some(database_url) = &self.postgres_database_url else {
            return Ok(());
        };
        let database_url = database_url.to_string();
        let on_payload: PayloadHandler = Arc::new(on_payload);
        std::thread::Builder::new()
            .name("scope-repo-change-listener".to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        tracing::error!(%error, "failed to start repo change listener runtime");
                        return;
                    }
                };
                runtime.block_on(async move {
                    loop {
                        if let Err(error) =
                            listen_for_repo_changes(&database_url, Arc::clone(&on_payload)).await
                        {
                            tracing::warn!(%error, "repo change listener disconnected");
                            tokio::time::sleep(RECONNECT_DELAY).await;
                        }
                    }
                });
            })?;
        Ok(())
    }

    pub fn queue_repo_change_notification(&self, payload: String) -> Result<(), PostgresError> {
        self.repo_change_notifier.queue(&self.db, payload)
    }
}

#[derive(Clone, Default)]
pub(in crate::db) struct RepoChangeNotifier {
    queue: Arc<OnceLock<mpsc::Sender<String>>>,
}

impl RepoChangeNotifier {
    fn queue(&self, db: &Arc<DatabaseConnection>, payload: String) -> Result<(), PostgresError> {
        let queue = self.queue.get_or_init(|| {
            let (queue, queued) = mpsc::channel(QUEUED_NOTIFICATION_LIMIT);
            tokio::spawn(send_queued_notifications(Arc::clone(db), queued));
            queue
        });
        queue.try_send(payload).map_err(|error| match error {
            mpsc::error::TrySendError::Full(_) => {
                PostgresError::internal_message("repo change notification queue is full")
            }
            mpsc::error::TrySendError::Closed(_) => {
                PostgresError::internal_message("repo change notification queue is closed")
            }
        })
    }
}

async fn send_queued_notifications(
    db: Arc<DatabaseConnection>,
    mut queued: mpsc::Receiver<String>,
) {
    let mut payloads = Vec::with_capacity(NOTIFICATIONS_PER_COMMIT);
    while queued
        .recv_many(&mut payloads, NOTIFICATIONS_PER_COMMIT)
        .await
        > 0
    {
        if let Err(error) = notify_together(db.as_ref(), &payloads).await {
            tracing::warn!(
                %error,
                notifications = payloads.len(),
                "failed to publish repo change notifications"
            );
        }
        payloads.clear();
    }
}

async fn notify_together(
    db: &DatabaseConnection,
    payloads: &[String],
) -> Result<(), sea_orm::DbErr> {
    let rows = (1..=payloads.len())
        .map(|index| format!("(${index})"))
        .collect::<Vec<_>>()
        .join(", ");
    db.execute_raw(Statement::from_sql_and_values(
        DbBackend::Postgres,
        format!(
            "SELECT pg_notify('{POSTGRES_REPO_CHANGE_CHANNEL}', payload)
             FROM (VALUES {rows}) AS queued(payload)"
        ),
        payloads.iter().cloned().map(sea_orm::Value::from),
    ))
    .await?;
    Ok(())
}

async fn listen_for_repo_changes(
    database_url: &str,
    on_payload: PayloadHandler,
) -> Result<(), sqlx::Error> {
    let mut listener = sqlx::postgres::PgListener::connect(database_url).await?;
    listener.listen(POSTGRES_REPO_CHANGE_CHANNEL).await?;
    loop {
        let notification = listener.recv().await?;
        on_payload(notification.payload().to_string());
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{MetadataStore, TestDatabaseTarget};
    use std::{sync::Mutex, time::Duration};

    #[tokio::test]
    async fn queued_notifications_reach_listeners_in_order() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let received = std::sync::Arc::new(Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&received);
        store
            .repositories()
            .start_repo_change_listener(move |payload| sink.lock().unwrap().push(payload))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let expected = (0..300)
            .map(|index| format!("change-{index}"))
            .collect::<Vec<_>>();
        for payload in &expected {
            store
                .repositories()
                .queue_repo_change_notification(payload.clone())
                .unwrap();
        }
        for _ in 0..100 {
            if received.lock().unwrap().len() >= expected.len() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(*received.lock().unwrap(), expected);
    }
}
