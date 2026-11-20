use super::RepositoryStore;
use crate::error::PostgresError;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};
use sqlx::postgres::PgListener;
use std::{
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

pub(crate) const POSTGRES_REPO_CHANGE_CHANNEL: &str = "scope_repo_changes";
const NOTIFICATIONS_LOST_PAYLOAD: &str = "notifications-lost";
const RECONNECT_DELAY: Duration = Duration::from_secs(2);
const LOSS_ANNOUNCEMENT_RETRY_DELAY: Duration = Duration::from_secs(1);
const QUEUED_NOTIFICATION_LIMIT: usize = 16_384;
const NOTIFICATIONS_PER_COMMIT: usize = 256;

pub struct RepoChangeListener {
    task: Mutex<Option<JoinHandle<()>>>,
}

impl RepoChangeListener {
    pub async fn shutdown(&self) {
        let task = self
            .task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(task) = task {
            task.abort();
            if let Err(error) = task.await
                && error.is_panic()
            {
                tracing::error!(%error, "repo change listener panicked");
            }
        }
    }
}

impl Drop for RepoChangeListener {
    fn drop(&mut self) {
        if let Some(task) = self
            .task
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            task.abort();
        }
    }
}

impl RepositoryStore {
    pub async fn listen_for_repo_changes(
        &self,
        on_payload: impl Fn(String) + Send + 'static,
        on_notifications_lost: impl Fn() + Send + 'static,
    ) -> Result<RepoChangeListener, PostgresError> {
        let database_url = self.postgres_database_url.clone().ok_or_else(|| {
            PostgresError::internal_message("repo change listener requires a PostgreSQL URL")
        })?;
        let listener = subscribe(&database_url).await.map_err(|error| {
            PostgresError::unavailable(format!("subscribing to repo changes: {error}"))
        })?;
        let task = tokio::spawn(receive_repo_changes(
            database_url,
            listener,
            on_payload,
            on_notifications_lost,
        ));
        Ok(RepoChangeListener {
            task: Mutex::new(Some(task)),
        })
    }

    pub fn queue_repo_change_notification(&self, payload: String) -> Result<(), PostgresError> {
        self.repo_change_notifier.queue(&self.db, payload)
    }
}

#[derive(Clone, Default)]
pub(in crate::db) struct RepoChangeNotifier {
    queue: Arc<OnceLock<mpsc::Sender<String>>>,
    lost: Arc<AtomicBool>,
}

impl RepoChangeNotifier {
    fn queue(&self, db: &Arc<DatabaseConnection>, payload: String) -> Result<(), PostgresError> {
        let queue = self.queue.get_or_init(|| {
            let (queue, queued) = mpsc::channel(QUEUED_NOTIFICATION_LIMIT);
            tokio::spawn(send_queued_notifications(
                Arc::clone(db),
                queued,
                Arc::clone(&self.lost),
            ));
            queue
        });
        queue.try_send(payload).map_err(|error| {
            self.lost.store(true, Ordering::Release);
            match error {
                mpsc::error::TrySendError::Full(_) => {
                    PostgresError::internal_message("repo change notification queue is full")
                }
                mpsc::error::TrySendError::Closed(_) => {
                    PostgresError::internal_message("repo change notification queue is closed")
                }
            }
        })
    }
}

async fn send_queued_notifications(
    db: Arc<DatabaseConnection>,
    mut queued: mpsc::Receiver<String>,
    lost: Arc<AtomicBool>,
) {
    let mut payloads = Vec::with_capacity(NOTIFICATIONS_PER_COMMIT);
    loop {
        let closed = if lost.load(Ordering::Acquire) {
            matches!(
                tokio::time::timeout(
                    LOSS_ANNOUNCEMENT_RETRY_DELAY,
                    queued.recv_many(&mut payloads, NOTIFICATIONS_PER_COMMIT),
                )
                .await,
                Ok(0)
            )
        } else {
            queued
                .recv_many(&mut payloads, NOTIFICATIONS_PER_COMMIT)
                .await
                == 0
        };
        if closed {
            return;
        }
        if lost.swap(false, Ordering::AcqRel) {
            payloads.push(NOTIFICATIONS_LOST_PAYLOAD.to_owned());
        }
        if let Err(error) = notify_together(db.as_ref(), &payloads).await {
            lost.store(true, Ordering::Release);
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

async fn subscribe(database_url: &str) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect(database_url).await?;
    listener.listen(POSTGRES_REPO_CHANGE_CHANNEL).await?;
    Ok(listener)
}

async fn receive_repo_changes(
    database_url: Arc<str>,
    mut listener: PgListener,
    on_payload: impl Fn(String),
    on_notifications_lost: impl Fn(),
) {
    loop {
        match listener.recv().await {
            Ok(notification) if notification.payload() == NOTIFICATIONS_LOST_PAYLOAD => {
                on_notifications_lost();
            }
            Ok(notification) => on_payload(notification.payload().to_string()),
            Err(error) => {
                tracing::warn!(%error, "repo change listener disconnected");
                listener = resubscribe(&database_url).await;
                on_notifications_lost();
            }
        }
    }
}

async fn resubscribe(database_url: &str) -> PgListener {
    loop {
        tokio::time::sleep(RECONNECT_DELAY).await;
        match subscribe(database_url).await {
            Ok(listener) => return listener,
            Err(error) => tracing::warn!(%error, "repo change listener failed to resubscribe"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::db::{MetadataStore, RepositoryStore, TestDatabaseTarget};
    use std::{sync::Arc, time::Duration};
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn listener_is_ready_only_once_subscribed_and_stops_on_shutdown() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let unreachable = RepositoryStore {
            postgres_database_url: Some(Arc::from("postgres://scope@127.0.0.1:1/scope")),
            ..store.repositories()
        };
        assert!(
            unreachable
                .listen_for_repo_changes(|_| {}, || {})
                .await
                .is_err()
        );

        let (received, mut payloads) = mpsc::unbounded_channel();
        let losses = received.clone();
        let listener = store
            .repositories()
            .listen_for_repo_changes(
                move |payload| received.send(payload).unwrap(),
                move || losses.send("lost".to_string()).unwrap(),
            )
            .await
            .unwrap();
        store
            .repositories()
            .queue_repo_change_notification("ready".to_string())
            .unwrap();
        let payload = tokio::time::timeout(Duration::from_secs(4), payloads.recv())
            .await
            .unwrap();
        assert_eq!(payload.as_deref(), Some("ready"));
        store
            .repositories()
            .queue_repo_change_notification(super::NOTIFICATIONS_LOST_PAYLOAD.to_string())
            .unwrap();
        let announced = tokio::time::timeout(Duration::from_secs(4), payloads.recv())
            .await
            .unwrap();
        assert_eq!(announced.as_deref(), Some("lost"));

        listener.shutdown().await;
        assert_eq!(
            payloads.try_recv(),
            Err(mpsc::error::TryRecvError::Disconnected)
        );
    }

    #[tokio::test]
    async fn queued_notifications_reach_listeners_in_order() {
        let target = TestDatabaseTarget::required().unwrap();
        let store = MetadataStore::connect_fresh_for_tests(&target).unwrap();
        let mut listener =
            sqlx::postgres::PgListener::connect_with(store.db.get_postgres_connection_pool())
                .await
                .unwrap();
        listener
            .listen(super::POSTGRES_REPO_CHANGE_CHANNEL)
            .await
            .unwrap();
        let expected = (0..300)
            .map(|index| format!("change-{index}"))
            .collect::<Vec<_>>();
        for payload in &expected {
            store
                .repositories()
                .queue_repo_change_notification(payload.clone())
                .unwrap();
        }
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            let mut received = Vec::with_capacity(expected.len());
            for _ in &expected {
                received.push(listener.recv().await.unwrap().payload().to_string());
            }
            received
        })
        .await
        .unwrap();
        assert_eq!(received, expected);
    }
}
