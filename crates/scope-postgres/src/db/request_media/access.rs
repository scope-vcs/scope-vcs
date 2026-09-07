use super::{
    AuthorizedRequestAttachment, MediaStore, RequestMediaManifest, RequestMediaObjectTarget,
    persistence::{attachment_by_id, bindings_for_attachment, manifest_by_id},
};
use crate::{
    db::{
        repository_access::repository_access, request_invitees::request_is_invitee,
        request_rows::request_by_id,
    },
    error::PostgresError,
};
use scope_domain::requests::attachments::{RequestAttachmentState, can_view_request_attachment};
use scope_domain::requests::{RequestViewer, request_policy};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl MediaStore {
    pub async fn request_attachment_for_viewer(
        &self,
        request_id: &str,
        attachment_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<AuthorizedRequestAttachment>, PostgresError> {
        authorized_attachment(self.db.as_ref(), request_id, attachment_id, viewer_user_id).await
    }

    pub async fn list_request_attachments_for_viewer(
        &self,
        request_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Vec<AuthorizedRequestAttachment>, PostgresError> {
        let ids = self
            .db
            .query_all(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "SELECT id FROM scope_request_media_attachments
                 WHERE request_id = $1 ORDER BY created_at_unix, id",
                [request_id.into()],
            ))
            .await
            .map_err(PostgresError::internal)?
            .into_iter()
            .map(|row| {
                row.try_get::<String>("", "id")
                    .map_err(PostgresError::internal)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut visible = Vec::new();
        for attachment_id in ids {
            if let Some(attachment) =
                authorized_attachment(self.db.as_ref(), request_id, &attachment_id, viewer_user_id)
                    .await?
            {
                visible.push(attachment);
            }
        }
        Ok(visible)
    }

    pub async fn authorized_media_manifest(
        &self,
        request_id: &str,
        attachment_id: &str,
        viewer_user_id: Option<&str>,
        target: RequestMediaObjectTarget<'_>,
    ) -> Result<Option<RequestMediaManifest>, PostgresError> {
        let Some(authorized) =
            authorized_attachment(self.db.as_ref(), request_id, attachment_id, viewer_user_id)
                .await?
        else {
            return Ok(None);
        };
        let manifest_id = match target {
            RequestMediaObjectTarget::Original if authorized.attachment.original_is_grantable() => {
                authorized
                    .attachment
                    .original
                    .as_ref()
                    .map(|object| object.object_key.as_str())
            }
            RequestMediaObjectTarget::Derivative(derivative_id)
                if authorized.attachment.state == RequestAttachmentState::Ready =>
            {
                authorized
                    .attachment
                    .derivatives
                    .iter()
                    .find(|derivative| derivative.id == derivative_id)
                    .map(|derivative| derivative.object.object_key.as_str())
            }
            _ => None,
        };
        let Some(manifest_id) = manifest_id else {
            return Ok(None);
        };
        let mut manifest = manifest_by_id(self.db.as_ref(), manifest_id).await?;
        if matches!(target, RequestMediaObjectTarget::Original)
            && let Some(manifest) = manifest.as_mut()
            && let Some(detected_media_type) = authorized.attachment.detected_media_type
        {
            // The declared type is retained in the immutable upload manifest. Once the
            // worker validates the source, serving uses the detected type.
            manifest.media_type = detected_media_type;
        }
        Ok(manifest)
    }
}

async fn authorized_attachment<C>(
    conn: &C,
    request_id: &str,
    attachment_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<AuthorizedRequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    if cleanup_tombstone_exists(conn, attachment_id).await? {
        return Ok(None);
    }
    let Some(attachment) = attachment_by_id(conn, attachment_id).await? else {
        return Ok(None);
    };
    if attachment.request_id != request_id {
        return Ok(None);
    }
    let Some(request) = request_by_id(conn, request_id).await? else {
        return Ok(None);
    };
    if request.repo_id != attachment.repository_id {
        return Ok(None);
    }
    let Some(repo) = repository_access(conn, &request.repo_id, viewer_user_id).await? else {
        return Ok(None);
    };
    let is_invitee = match viewer_user_id {
        Some(user_id) => request_is_invitee(conn, request_id, user_id).await?,
        None => false,
    };
    let policy = request_policy(
        &request,
        RequestViewer::new(repo.access, viewer_user_id, is_invitee),
    );
    // Upload ownership never outlives current access to the request itself.
    if !policy.exact_visible {
        return Ok(None);
    }
    let bindings = bindings_for_attachment(conn, attachment_id).await?;
    if !can_view_request_attachment(
        &attachment,
        &bindings,
        viewer_user_id,
        policy.exact_visible,
        policy.discussion_visible,
    ) {
        return Ok(None);
    }
    Ok(Some(AuthorizedRequestAttachment {
        repository_id: attachment.repository_id.clone(),
        request_audience: request.audience,
        attachment,
        bindings,
    }))
}

pub(super) async fn cleanup_tombstone_exists<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(conn
        .query_one(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_request_media_cleanup_jobs WHERE attachment_id = $1",
            [attachment_id.into()],
        ))
        .await
        .map_err(PostgresError::internal)?
        .is_some())
}
