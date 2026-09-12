use super::{
    access::cleanup_tombstone_exists,
    locks::lock_attachment,
    persistence::{binding_target_parts, bindings_for_attachment},
};
use crate::error::PostgresError;
use scope_domain::requests::attachments::{
    RequestAttachmentBindingTarget, RequestAttachmentLimits, replace_attachment_bindings,
};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use std::collections::BTreeSet;

pub(in crate::db) async fn replace_bindings_for_markdown<C>(
    conn: &C,
    request_id: &str,
    actor_user_id: &str,
    target: &RequestAttachmentBindingTarget,
    markdown: &str,
    now_unix: u64,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let referenced = scope_domain::requests::attachments::request_attachment_references(markdown)
        .map_err(PostgresError::from)?;
    let limits = RequestAttachmentLimits::default();
    if referenced.len() > limits.max_attachments_per_content {
        return Err(PostgresError::invalid_input(format!(
            "request content may reference at most {} attachments",
            limits.max_attachments_per_content
        )));
    }
    let (target_key, target_kind, discussion_id, reply_id) = binding_target_parts(target);
    let existing_ids = existing_target_bindings(conn, request_id, &target_key, false).await?;
    let mut lock_ids = existing_ids.union(&referenced).cloned().collect::<Vec<_>>();
    lock_ids.sort();

    let mut attachments = Vec::new();
    let mut existing_bindings = Vec::new();
    for attachment_id in &lock_ids {
        let attachment = lock_attachment(conn, attachment_id).await?;
        existing_bindings.extend(bindings_for_attachment(conn, attachment_id).await?);
        attachments.push(attachment);
    }

    let locked_existing_ids = existing_target_bindings(conn, request_id, &target_key, true).await?;
    if locked_existing_ids != existing_ids {
        return Err(PostgresError::conflict(
            "request attachment bindings changed; retry the edit",
        ));
    }

    for attachment_id in &lock_ids {
        if cleanup_tombstone_exists(conn, attachment_id).await? {
            return Err(PostgresError::conflict(
                "request attachment is being deleted",
            ));
        }
    }

    let projected = replace_attachment_bindings(
        request_id,
        actor_user_id,
        true,
        target.clone(),
        markdown,
        &attachments,
        &existing_bindings,
    )?;

    conn.execute(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "DELETE FROM scope_request_media_bindings WHERE request_id = $1 AND target_key = $2",
        [request_id.into(), target_key.clone().into()],
    ))
    .await
    .map_err(PostgresError::internal)?;

    for binding in &projected {
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "INSERT INTO scope_request_media_bindings (
                attachment_id, request_id, target_key, target_kind,
                discussion_id, reply_id, bound_at_unix
             ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            [
                binding.attachment_id.clone().into(),
                request_id.into(),
                target_key.clone().into(),
                target_kind.into(),
                discussion_id.clone().into(),
                reply_id.clone().into(),
                super::persistence::as_i64(now_unix, "binding time")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_attachments SET unbound_expires_at_unix = NULL
             WHERE id = $1",
            [binding.attachment_id.clone().into()],
        ))
        .await
        .map_err(PostgresError::internal)?;
    }

    let unbound_at = now_unix
        .checked_add(limits.unbound_attachment_ttl_seconds)
        .ok_or_else(|| PostgresError::internal_message("attachment expiry overflow"))?;
    for attachment_id in existing_ids.difference(&referenced) {
        conn.execute(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE scope_request_media_attachments attachment
             SET unbound_expires_at_unix = $2
             WHERE attachment.id = $1
               AND NOT EXISTS (
                    SELECT 1 FROM scope_request_media_bindings binding
                    WHERE binding.attachment_id = attachment.id
               )",
            [
                attachment_id.clone().into(),
                super::persistence::as_i64(unbound_at, "unbound expiry")?.into(),
            ],
        ))
        .await
        .map_err(PostgresError::internal)?;
    }
    for attachment_id in existing_ids.symmetric_difference(&referenced) {
        let attachment = attachments
            .iter()
            .find(|attachment| &attachment.id == attachment_id)
            .expect("binding attachments were loaded and locked");
        super::processing::notify_attachment_change(conn, attachment).await?;
    }
    Ok(())
}

async fn existing_target_bindings<C>(
    conn: &C,
    request_id: &str,
    target_key: &str,
    lock: bool,
) -> Result<BTreeSet<String>, PostgresError>
where
    C: ConnectionTrait,
{
    conn.query_all(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        if lock {
            "SELECT attachment_id FROM scope_request_media_bindings
             WHERE request_id = $1 AND target_key = $2
             ORDER BY attachment_id FOR UPDATE"
        } else {
            "SELECT attachment_id FROM scope_request_media_bindings
             WHERE request_id = $1 AND target_key = $2
             ORDER BY attachment_id"
        },
        [request_id.into(), target_key.into()],
    ))
    .await
    .map_err(PostgresError::internal)?
    .into_iter()
    .map(|row| {
        row.try_get::<String>("", "attachment_id")
            .map_err(PostgresError::internal)
    })
    .collect::<Result<_, _>>()
}
