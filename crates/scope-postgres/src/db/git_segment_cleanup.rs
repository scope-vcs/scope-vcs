use sea_orm::{ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyGitSegmentObject {
    pub object_key: String,
    pub sha256: String,
}

pub struct GitSegmentV1Cleanup {
    db: DatabaseConnection,
}

impl GitSegmentV1Cleanup {
    pub async fn begin(database_url: String) -> anyhow::Result<Self> {
        let db = Database::connect(database_url).await?;
        crate::migrations::assert_exact_state(&db).await?;
        Ok(Self { db })
    }

    pub async fn legacy_objects(&self) -> anyhow::Result<Vec<LegacyGitSegmentObject>> {
        let referenced = scalar_i64(
            &self.db,
            "SELECT count(*) AS value FROM scope_object_references
             WHERE object_key::jsonb ? 'GitSegmentSha256'",
        )
        .await?;
        if referenced != 0 {
            anyhow::bail!("refusing to delete referenced legacy Git segment objects");
        }
        self.db
            .query_all(Statement::from_string(
                DatabaseBackend::Postgres,
                r#"
                SELECT object_key, sha256
                FROM scope_orphan_object_jobs
                WHERE object_key::jsonb =
                      jsonb_build_object('GitSegmentSha256', sha256)
                ORDER BY object_key
                "#
                .to_string(),
            ))
            .await?
            .into_iter()
            .map(|row| {
                Ok(LegacyGitSegmentObject {
                    object_key: required(&row, "object_key")?,
                    sha256: required(&row, "sha256")?,
                })
            })
            .collect()
    }

    pub async fn remove_record(&self, object: &LegacyGitSegmentObject) -> anyhow::Result<()> {
        self.db
            .execute(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "DELETE FROM scope_orphan_object_jobs
                 WHERE object_key = $1 AND sha256 = $2
                   AND object_key::jsonb = jsonb_build_object('GitSegmentSha256', sha256)",
                [
                    object.object_key.clone().into(),
                    object.sha256.clone().into(),
                ],
            ))
            .await?;
        Ok(())
    }
}

fn required(row: &sea_orm::QueryResult, column: &str) -> anyhow::Result<String> {
    Ok(row.try_get::<String>("", column)?)
}

async fn scalar_i64(db: &DatabaseConnection, sql: &str) -> anyhow::Result<i64> {
    Ok(db
        .query_one(Statement::from_string(
            DatabaseBackend::Postgres,
            sql.to_string(),
        ))
        .await?
        .ok_or_else(|| anyhow::anyhow!("database query returned no row"))?
        .try_get::<i64>("", "value")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{TestDatabaseTarget, test_support::connect_isolated_test_database};

    #[tokio::test]
    async fn retired_segment_cleanup_requires_current_schema_and_no_references() {
        let target = TestDatabaseTarget::required().unwrap();
        let (db, _lease) = connect_isolated_test_database(&target).await.unwrap();
        assert!(
            GitSegmentV1Cleanup::begin(target.schema_database_url())
                .await
                .is_err()
        );
        crate::migrations::apply_in_maintenance(&db, Default::default())
            .await
            .unwrap();
        db.execute_unprepared(
            r#"
            INSERT INTO scope_orphan_object_jobs (
                object_key, generation, sha256, git_oid, size_bytes,
                attempts, next_run_at_unix, created_at_unix, updated_at_unix
            ) VALUES (
                jsonb_build_object('GitSegmentSha256', repeat('a',64))::text,
                'retired-test', repeat('a',64), repeat('b',40), 1, 0, 1, 1, 1
            ), (
                jsonb_build_object('BlobSha256', repeat('c',64))::text,
                'unrelated-test', repeat('c',64), repeat('b',40), 1, 0, 1, 1, 1
            );
            INSERT INTO scope_object_references (object_key, ref_kind, ref_id)
            VALUES (jsonb_build_object('GitSegmentSha256', repeat('a',64))::text,
                    'run_source', 'retired-run');
        "#,
        )
        .await
        .unwrap();
        let cleanup = GitSegmentV1Cleanup::begin(target.schema_database_url())
            .await
            .unwrap();
        assert!(cleanup.legacy_objects().await.is_err());
        db.execute_unprepared("DELETE FROM scope_object_references WHERE ref_id = 'retired-run'")
            .await
            .unwrap();
        let objects = cleanup.legacy_objects().await.unwrap();
        assert_eq!(objects.len(), 1);
        cleanup.remove_record(&objects[0]).await.unwrap();
        assert!(cleanup.legacy_objects().await.unwrap().is_empty());
        assert_eq!(
            scalar_i64(
                &db,
                "SELECT count(*) AS value FROM scope_orphan_object_jobs"
            )
            .await
            .unwrap(),
            1
        );
    }
}
