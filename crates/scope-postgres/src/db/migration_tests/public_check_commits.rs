use super::*;

#[tokio::test]
async fn only_an_evaluation_that_tests_a_check_commit_names_what_it_merges() {
    let (_target, db, _lease) = isolated_database().await;
    migrations::apply_in_maintenance(db.as_ref(), Default::default())
        .await
        .unwrap();
    db.execute_unprepared(
        r#"
        INSERT INTO scope_users VALUES ('owner','owner','owner@scope.test',true);
        INSERT INTO scope_repositories (id,owner_handle,name,owner_user_id,publication_state,
            change_version,content_version,repo_config,policy,incarnation_id)
            VALUES ('owner/one','owner','one','owner','Ready',0,0,'{}','{}','repoi_m0071_one');
        INSERT INTO scope_requests (id,repo_id,name,author_user_id,author_role,audience,
            base_main_oid,head_oid,title,description_markdown,activity_version,
            created_at_unix,updated_at_unix)
            VALUES ('req_1','owner/one','one','owner','Owner','Public',
                    repeat('a',40),repeat('b',40),'one','',0,1,1);
        INSERT INTO scope_request_check_evaluations (request_id,head_oid,tested_oid,state,
            message,checks,created_at_unix,updated_at_unix,
            check_private_main_oid,check_public_base_oid)
            VALUES ('req_1',repeat('b',40),repeat('c',40),'awaiting-approval',NULL,
                    '[{"provider":"github","name":"ci / test"}]',2,2,
                    repeat('d',40),repeat('a',40));
        "#,
    )
    .await
    .unwrap();

    for invalid in [
        "UPDATE scope_request_check_evaluations SET check_public_base_oid = NULL",
        "UPDATE scope_request_check_evaluations
            SET check_private_main_oid = NULL, check_public_base_oid = NULL",
        "UPDATE scope_request_check_evaluations SET tested_oid = head_oid",
        "UPDATE scope_request_check_evaluations SET check_private_main_oid = 'short'",
    ] {
        assert!(db.execute_unprepared(invalid).await.is_err(), "{invalid}");
    }
    db.execute_unprepared(
        "UPDATE scope_request_check_evaluations
            SET tested_oid = head_oid, check_private_main_oid = NULL,
                check_public_base_oid = NULL",
    )
    .await
    .unwrap();
    migrations::assert_exact_state(db.as_ref()).await.unwrap();
}
