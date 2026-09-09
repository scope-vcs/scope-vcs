use super::*;

#[tokio::test]
async fn git_segment_schema_uses_segment_identity() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref()).await.unwrap();

    let columns = db
        .query_all(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT column_name
             FROM information_schema.columns
             WHERE table_schema = current_schema()
               AND table_name = 'scope_git_segments'
             ORDER BY column_name"
                .to_string(),
        ))
        .await
        .unwrap()
        .into_iter()
        .map(|row| row.try_get::<String>("", "column_name").unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        columns,
        [
            "base_oid",
            "first_sequence",
            "geometric_tier",
            "head_oid",
            "last_sequence",
            "repo_id",
            "segment_id",
        ]
    );
}
