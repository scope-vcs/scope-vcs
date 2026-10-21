use super::{
    MediaStore, RequestMediaObjectTarget,
    persistence::{
        attachments_for_request, bindings_for_request, live_attachment_with_bindings,
        manifest_by_id,
    },
};
use crate::{
    db::{
        repository_access::repository_access, request_invitees::request_is_invitee,
        request_rows::request_by_id,
    },
    error::PostgresError,
};
use scope_domain::requests::attachments::{
    RequestAttachment, RequestAttachmentState, RequestMediaObject, can_view_request_attachment,
};
use scope_domain::requests::{Request, RequestPolicyDecision, RequestViewer, request_policy};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

impl MediaStore {
    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_attachment_for_viewer"))]
    pub async fn request_attachment_for_viewer(
        &self,
        request_id: &str,
        attachment_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RequestAttachment>, PostgresError> {
        let tx = crate::db::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let attachment =
            authorized_attachment(&tx, request_id, attachment_id, viewer_user_id).await?;
        tx.commit().await?;
        Ok(attachment)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "list_request_attachments_for_viewer"))]
    pub async fn list_request_attachments_for_viewer(
        &self,
        request_id: &str,
        viewer_user_id: Option<&str>,
    ) -> Result<Vec<RequestAttachment>, PostgresError> {
        let tx = crate::db::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some((request, policy)) =
            visible_request_policy(&tx, request_id, viewer_user_id).await?
        else {
            return Ok(Vec::new());
        };
        let attachments = attachments_for_request(&tx, request_id).await?;
        let mut bindings = bindings_for_request(&tx, request_id).await?;
        let visible = attachments
            .into_iter()
            .filter_map(|attachment| {
                let attachment_bindings = bindings.remove(&attachment.id).unwrap_or_default();
                (attachment.repository_id == request.repo_id
                    && can_view_request_attachment(
                        &attachment,
                        &attachment_bindings,
                        viewer_user_id,
                        policy.exact_visible,
                        policy.discussion_visible,
                    ))
                .then_some(attachment)
            })
            .collect();
        tx.commit().await?;
        Ok(visible)
    }

    #[tracing::instrument(skip_all, fields(otel.kind = "client", db.system.name = "postgresql", db.operation.name = "request_attachment_media_for_viewer"))]
    pub async fn request_attachment_media_for_viewer(
        &self,
        request_id: &str,
        attachment_id: &str,
        viewer_user_id: Option<&str>,
        target: RequestMediaObjectTarget<'_>,
    ) -> Result<Option<(RequestAttachment, Option<RequestMediaObject>)>, PostgresError> {
        let tx = crate::db::begin_metadata_read_snapshot(self.db.as_ref()).await?;
        let Some(attachment) =
            authorized_attachment(&tx, request_id, attachment_id, viewer_user_id).await?
        else {
            return Ok(None);
        };
        let manifest_id = match target {
            RequestMediaObjectTarget::Original if attachment.original_is_grantable() => attachment
                .original
                .as_ref()
                .map(|object| object.object_key.as_str()),
            RequestMediaObjectTarget::Derivative(derivative_id)
                if attachment.state == RequestAttachmentState::Ready =>
            {
                attachment
                    .derivatives
                    .iter()
                    .find(|derivative| derivative.id == derivative_id)
                    .map(|derivative| derivative.object.object_key.as_str())
            }
            _ => None,
        };
        let mut object = match manifest_id {
            Some(manifest_id) => manifest_by_id(&tx, manifest_id).await?,
            None => None,
        };
        tx.commit().await?;
        if matches!(target, RequestMediaObjectTarget::Original)
            && let Some(object) = object.as_mut()
            && let Some(detected_media_type) = &attachment.detected_media_type
        {
            object.media_type = detected_media_type.clone();
        }
        Ok(Some((attachment, object)))
    }
}

async fn visible_request_policy<C>(
    conn: &C,
    request_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<(Request, RequestPolicyDecision)>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(request) = request_by_id(conn, request_id).await? else {
        return Ok(None);
    };
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
        &repo.views,
    );
    Ok(policy.exact_visible.then_some((request, policy)))
}

async fn authorized_attachment<C>(
    conn: &C,
    request_id: &str,
    attachment_id: &str,
    viewer_user_id: Option<&str>,
) -> Result<Option<RequestAttachment>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some((attachment, bindings)) = live_attachment_with_bindings(conn, attachment_id).await?
    else {
        return Ok(None);
    };
    if attachment.request_id != request_id {
        return Ok(None);
    }
    let Some((request, policy)) = visible_request_policy(conn, request_id, viewer_user_id).await?
    else {
        return Ok(None);
    };
    Ok((request.repo_id == attachment.repository_id
        && can_view_request_attachment(
            &attachment,
            &bindings,
            viewer_user_id,
            policy.exact_visible,
            policy.discussion_visible,
        ))
    .then_some(attachment))
}

pub(super) async fn cleanup_tombstone_exists<C>(
    conn: &C,
    attachment_id: &str,
) -> Result<bool, PostgresError>
where
    C: ConnectionTrait,
{
    Ok(conn
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT 1 AS present FROM scope_request_media_cleanup_jobs WHERE attachment_id = $1",
            [attachment_id.into()],
        ))
        .await?
        .is_some())
}
