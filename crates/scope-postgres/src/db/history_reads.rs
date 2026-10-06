use super::integer_columns::{self, usize_to_i64};
use super::projection_read_models::{fold_live_projection_read_models, live_projection_read_model};
use super::{RepositoryStore, acquire_aggregate_lock, begin_metadata_read_snapshot, entities};
use crate::error::PostgresError;
use scope_domain::{
    history::{HistoryEntry, HistoryFeed, HistoryView},
    projection::ProjectionViewKey,
    repository::RepositoryIncarnation,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement, TransactionTrait};
use sha2::{Digest, Sha256};

pub struct RepositoryHistoryQuery<'a> {
    pub incarnation: &'a RepositoryIncarnation,
    pub change_version: u64,
    pub audience: ProjectionViewKey,
    pub feed: HistoryFeed,
    pub before: Option<&'a RepositoryHistoryBoundary>,
    pub entry_source_id: Option<&'a str>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepositoryHistoryBoundary {
    pub generation: String,
    pub position: u64,
}

pub struct RepositoryHistoryPage {
    pub view: HistoryView,
    pub neighbors: Option<RepositoryHistoryNeighbors>,
    pub head_oid: Option<String>,
    pub next_boundary: Option<RepositoryHistoryBoundary>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RepositoryHistoryNeighbors {
    pub older_source_id: Option<String>,
    pub newer_source_id: Option<String>,
}

/// Appends a view's history entries from `first_position`. Each distinct
/// payload is stored once per repository; views that render an entry the same
/// way share it.
pub(super) async fn append_history_entries<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    view: ProjectionViewKey,
    first_position: usize,
    entries: &[HistoryEntry],
) -> Result<(), PostgresError> {
    for (batch_index, batch) in entries.chunks(500).enumerate() {
        let mut payload_values = Vec::with_capacity(batch.len() * 3);
        let mut entry_values = Vec::with_capacity(batch.len() * 5);
        let mut payload_rows = Vec::with_capacity(batch.len());
        let mut entry_rows = Vec::with_capacity(batch.len());
        for (index, entry) in batch.iter().enumerate() {
            let payload = serde_json::to_value(entry).map_err(PostgresError::internal)?;
            let payload_hash = hex::encode(Sha256::digest(
                serde_json::to_vec(&payload).map_err(PostgresError::internal)?,
            ));
            let position = usize_to_i64(
                first_position + batch_index * 500 + index,
                "history position",
            )?;
            let offset = index * 3;
            payload_values.extend([repo_id.into(), payload_hash.clone().into(), payload.into()]);
            payload_rows.push(format!("(${},${},${})", offset + 1, offset + 2, offset + 3));
            let offset = index * 5;
            entry_values.extend([
                repo_id.into(),
                view.as_str().into(),
                position.into(),
                entry.source_id.clone().into(),
                payload_hash.into(),
            ]);
            entry_rows.push(format!(
                "(${},${},${},${},${})",
                offset + 1,
                offset + 2,
                offset + 3,
                offset + 4,
                offset + 5
            ));
        }
        conn.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            format!("INSERT INTO scope_repository_history_payloads (repo_id,payload_hash,payload) VALUES {} ON CONFLICT DO NOTHING", payload_rows.join(",")), payload_values,
        )).await.map_err(PostgresError::internal)?;
        conn.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            format!("INSERT INTO scope_repository_history_entries (repo_id,audience,position,source_id,payload_hash) VALUES {}", entry_rows.join(",")), entry_values,
        )).await.map_err(PostgresError::internal)?;
    }
    Ok(())
}

pub(super) async fn delete_history_payloads<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
) -> Result<(), PostgresError> {
    conn.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_repository_history_payloads WHERE repo_id=$1",
        [repo_id.into()],
    ))
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}

impl RepositoryStore {
    /// Builds the read models for the repository's current content when a
    /// read finds them missing or stale.
    pub async fn ensure_live_projection_read_models(
        &self,
        incarnation: &RepositoryIncarnation,
    ) -> Result<(), PostgresError> {
        let tx = self.db.begin().await.map_err(PostgresError::internal)?;
        acquire_aggregate_lock(&tx, "repository", incarnation.repository_id()).await?;
        let row = entities::repository::Entity::find_by_id(incarnation.repository_id())
            .one(&tx)
            .await
            .map_err(PostgresError::internal)?
            .ok_or_else(|| PostgresError::not_found("repo not found"))?;
        if row.incarnation_id != incarnation.incarnation_id() {
            return Err(PostgresError::conflict(
                "repository was recreated; retry the read",
            ));
        }
        let version =
            integer_columns::i64_to_u64(row.content_version, "repository content version")?;
        let mut missing = false;
        for view in [ProjectionViewKey::Private, ProjectionViewKey::Public] {
            missing |= live_projection_read_model(&tx, &row.id, version, view)
                .await?
                .is_none();
        }
        if missing {
            fold_live_projection_read_models(&tx, &row.id, version).await?;
        }
        tx.commit().await.map_err(PostgresError::internal)
    }

    pub async fn repository_history_page(
        &self,
        query: RepositoryHistoryQuery<'_>,
    ) -> Result<RepositoryHistoryPage, PostgresError> {
        let RepositoryHistoryQuery {
            incarnation,
            change_version,
            audience,
            feed,
            before,
            entry_source_id,
            limit,
        } = query;
        for _ in 0..2 {
            let tx = begin_metadata_read_snapshot(self.db.as_ref()).await?;
            let current =
                super::repository_access::repository_access(&tx, incarnation.repository_id(), None)
                    .await?
                    .ok_or_else(|| PostgresError::not_found("repo not found"))?;
            if current.incarnation() != *incarnation
                || current.record.change_version != change_version
            {
                return Err(PostgresError::conflict(
                    "repository changed while reading history; retry",
                ));
            }
            let Some(metadata) = live_projection_read_model(
                &tx,
                incarnation.repository_id(),
                current.record.content_version,
                audience,
            )
            .await?
            else {
                tx.commit().await.map_err(PostgresError::internal)?;
                self.ensure_live_projection_read_models(incarnation).await?;
                continue;
            };
            let generation = feed.generation(
                &metadata.history_generation,
                incarnation.repository_id(),
                audience.as_str(),
            );
            let boundary = match before {
                Some(boundary) => {
                    if boundary.generation != generation {
                        return Err(PostgresError::invalid_input(
                            "history changed; restart pagination",
                        ));
                    }
                    let position =
                        integer_columns::u64_to_i64(boundary.position, "history position")?;
                    tx.query_one_raw(Statement::from_sql_and_values(
                        DatabaseBackend::Postgres,
                        "SELECT position FROM scope_repository_history_entries WHERE repo_id=$1 AND audience=$2 AND position=$3",
                        [incarnation.repository_id().into(), audience.as_str().into(), position.into()],
                    )).await.map_err(PostgresError::internal)?
                        .ok_or_else(|| PostgresError::invalid_input("history cursor boundary is no longer available"))?;
                    Some(position)
                }
                None => None,
            };
            let limit = limit.clamp(1, 50) as i64;
            let mut values = vec![incarnation.repository_id().into(), audience.as_str().into()];
            let feed_predicate = match feed {
                HistoryFeed::Updates => " AND p.payload->>'kind' != 'VisibilityChange'",
                HistoryFeed::All => "",
                HistoryFeed::Visibility => " AND p.payload->'visibility_changes' != '[]'::jsonb",
            };
            let sql = if let Some(source_id) = entry_source_id {
                values.push(source_id.into());
                "SELECT e.position, p.payload, \
                    (SELECT o.source_id FROM scope_repository_history_entries o WHERE o.repo_id=e.repo_id AND o.audience=e.audience AND o.position<e.position ORDER BY o.position DESC LIMIT 1) AS older_source_id, \
                    (SELECT n.source_id FROM scope_repository_history_entries n WHERE n.repo_id=e.repo_id AND n.audience=e.audience AND n.position>e.position ORDER BY n.position LIMIT 1) AS newer_source_id \
                 FROM scope_repository_history_entries e JOIN scope_repository_history_payloads p ON p.repo_id=e.repo_id AND p.payload_hash=e.payload_hash \
                 WHERE e.repo_id=$1 AND e.audience=$2 AND e.source_id=$3".to_string()
            } else if let Some(position) = boundary {
                values.extend([position.into(), (limit + 1).into()]);
                format!(
                    "SELECT e.position, p.payload FROM scope_repository_history_entries e JOIN scope_repository_history_payloads p ON p.repo_id=e.repo_id AND p.payload_hash=e.payload_hash WHERE e.repo_id=$1 AND e.audience=$2 AND e.position<$3{feed_predicate} ORDER BY e.position DESC LIMIT $4"
                )
            } else {
                values.push((limit + 1).into());
                format!(
                    "SELECT e.position, p.payload FROM scope_repository_history_entries e JOIN scope_repository_history_payloads p ON p.repo_id=e.repo_id AND p.payload_hash=e.payload_hash WHERE e.repo_id=$1 AND e.audience=$2{feed_predicate} ORDER BY e.position DESC LIMIT $3"
                )
            };
            let rows = tx
                .query_all_raw(Statement::from_sql_and_values(
                    DatabaseBackend::Postgres,
                    sql,
                    values,
                ))
                .await
                .map_err(PostgresError::internal)?;
            let next_boundary = if rows.len() > limit as usize {
                Some(RepositoryHistoryBoundary {
                    generation: generation.clone(),
                    position: integer_columns::i64_to_u64(
                        rows[limit as usize - 1]
                            .try_get("", "position")
                            .map_err(PostgresError::internal)?,
                        "history position",
                    )?,
                })
            } else {
                None
            };
            let neighbors = match (entry_source_id, rows.first()) {
                (Some(_), Some(row)) => Some(RepositoryHistoryNeighbors {
                    older_source_id: row
                        .try_get("", "older_source_id")
                        .map_err(PostgresError::internal)?,
                    newer_source_id: row
                        .try_get("", "newer_source_id")
                        .map_err(PostgresError::internal)?,
                }),
                _ => None,
            };
            let mut entries = rows
                .into_iter()
                .map(|row| {
                    serde_json::from_value::<HistoryEntry>(
                        row.try_get("", "payload")
                            .map_err(PostgresError::internal)?,
                    )
                    .map_err(PostgresError::internal)
                })
                .collect::<Result<Vec<_>, _>>()?;
            entries.truncate(limit as usize);
            tx.commit().await.map_err(PostgresError::internal)?;
            return Ok(RepositoryHistoryPage {
                view: HistoryView {
                    repo_id: incarnation.repository_id().to_string(),
                    view_key: audience.as_str().to_string(),
                    generation,
                    entries,
                },
                neighbors,
                next_boundary,
                head_oid: metadata.head_oid,
            });
        }
        Err(PostgresError::conflict(
            "repository changed while reading history; retry",
        ))
    }
}

#[cfg(test)]
mod tests;
