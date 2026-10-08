use super::{acquire_aggregate_lock, entities, repository_access::load_repo_record};
use crate::error::PostgresError;
use scope_domain::repository::collaboration::{
    CollaborationState, RepositoryCollaboration, RepositoryInvite, RepositoryMember,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseTransaction, EntityTrait,
    IntoActiveModel, QueryFilter, QueryOrder, sea_query::Expr,
};
use std::collections::BTreeMap;

pub(super) async fn lock_collaboration_state(
    tx: &DatabaseTransaction,
    repo_id: &str,
) -> Result<Option<CollaborationState>, PostgresError> {
    acquire_aggregate_lock(tx, "repository", repo_id).await?;
    load_collaboration_state(tx, repo_id).await
}

pub(super) async fn load_collaboration_state<C>(
    conn: &C,
    repo_id: &str,
) -> Result<Option<CollaborationState>, PostgresError>
where
    C: ConnectionTrait,
{
    let Some(record) = load_repo_record(conn, repo_id).await? else {
        return Ok(None);
    };
    let views = super::projection_read_models::repository_views(conn, repo_id).await?;
    let collaboration = load_repository_collaboration(conn, repo_id).await?;
    Ok(Some(CollaborationState {
        record,
        views,
        collaboration,
    }))
}

pub(super) async fn save_collaboration_state<C>(
    conn: &C,
    before: &CollaborationState,
    after: &CollaborationState,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let mut unchanged_record = after.record.clone();
    unchanged_record.change_version = before.record.change_version;
    if unchanged_record != before.record {
        return Err(PostgresError::internal_message(
            "a collaboration change can only advance the repository change version",
        ));
    }
    save_repository_collaboration_delta(
        conn,
        &after.record.id,
        &before.collaboration,
        &after.collaboration,
    )
    .await?;
    if after.record.change_version == before.record.change_version {
        return Ok(());
    }
    use entities::repository::{Column, Entity};
    let updated = Entity::update_many()
        .col_expr(
            Column::ChangeVersion,
            Expr::value(super::integer_columns::u64_to_i64(
                after.record.change_version,
                "repository change version",
            )?),
        )
        .filter(Column::Id.eq(after.record.id.as_str()))
        .filter(Column::IncarnationId.eq(after.record.incarnation_id.as_str()))
        .filter(Column::ChangeVersion.eq(super::integer_columns::u64_to_i64(
            before.record.change_version,
            "repository change version",
        )?))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
    if updated.rows_affected != 1 {
        return Err(PostgresError::internal_message(
            "repository changed under its lock during a collaboration change",
        ));
    }
    let previously_assigned = before
        .collaboration
        .members
        .iter()
        .map(|member| &member.permissions.view)
        .collect::<std::collections::BTreeSet<_>>();
    let newly_assigned = after
        .collaboration
        .members
        .iter()
        .map(|member| &member.permissions.view)
        .filter(|view| !previously_assigned.contains(view))
        .collect::<std::collections::BTreeSet<_>>();
    for view in newly_assigned {
        if super::projection_read_models::live_projection_read_model(
            conn,
            &after.record.id,
            after.record.content_version,
            view,
        )
        .await?
        .is_none()
        {
            super::projection_read_models::build_projection_read_model(
                conn,
                &after.record.id,
                after.record.content_version,
                view,
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn load_repository_collaboration<C>(
    conn: &C,
    repo_id: &str,
) -> Result<RepositoryCollaboration, PostgresError>
where
    C: ConnectionTrait,
{
    let members = entities::repository_member::Entity::find()
        .filter(entities::repository_member::Column::RepoId.eq(repo_id))
        .order_by_asc(entities::repository_member::Column::UserId)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?
        .into_iter()
        .map(entities::repository_member::Model::try_into_domain)
        .collect::<Result<Vec<RepositoryMember>, _>>()?;
    let invite_rows = entities::repository_invite::Entity::find()
        .filter(entities::repository_invite::Column::RepoId.eq(repo_id))
        .order_by_asc(entities::repository_invite::Column::InvitedEmailNormalized)
        .order_by_asc(entities::repository_invite::Column::Id)
        .all(conn)
        .await
        .map_err(PostgresError::internal)?;
    let mut link_hashes = BTreeMap::<String, Vec<String>>::new();
    if !invite_rows.is_empty() {
        let links = entities::repository_invite_link::Entity::find()
            .filter(
                entities::repository_invite_link::Column::InviteId
                    .is_in(invite_rows.iter().map(|invite| invite.id.clone())),
            )
            .order_by_asc(entities::repository_invite_link::Column::TokenHash)
            .all(conn)
            .await
            .map_err(PostgresError::internal)?;
        for link in links {
            link_hashes
                .entry(link.invite_id)
                .or_default()
                .push(link.token_hash);
        }
    }
    let invitations = invite_rows
        .into_iter()
        .map(|invite| {
            let hashes = link_hashes.remove(&invite.id).unwrap_or_default();
            invite.try_into_domain(hashes)
        })
        .collect::<Result<Vec<RepositoryInvite>, _>>()?;
    Ok(RepositoryCollaboration {
        members,
        invitations,
    })
}

pub(super) async fn insert_repository_collaboration<C>(
    conn: &C,
    collaboration: &RepositoryCollaboration,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    for member in &collaboration.members {
        entities::repository_member::Model::from_domain(member)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(PostgresError::internal)?;
    }

    for invite in &collaboration.invitations {
        insert_repository_invite(conn, invite).await?;
    }

    Ok(())
}

pub(super) async fn save_repository_collaboration_delta<C>(
    conn: &C,
    repo_id: &str,
    before: &RepositoryCollaboration,
    after: &RepositoryCollaboration,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    let before_members = before
        .members
        .iter()
        .map(|member| (member.user_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    let after_members = after
        .members
        .iter()
        .map(|member| (member.user_id.as_str(), member))
        .collect::<BTreeMap<_, _>>();
    for user_id in before_members.keys() {
        if !after_members.contains_key(user_id) {
            entities::repository_member::Entity::delete_by_id((
                repo_id.to_string(),
                (*user_id).to_string(),
            ))
            .exec(conn)
            .await
            .map_err(PostgresError::internal)?;
        }
    }
    for (user_id, member) in after_members {
        if before_members
            .get(user_id)
            .is_some_and(|old| *old == member)
        {
            continue;
        }
        entities::repository_member::Entity::delete_by_id((
            repo_id.to_string(),
            user_id.to_string(),
        ))
        .exec(conn)
        .await
        .map_err(PostgresError::internal)?;
        entities::repository_member::Model::from_domain(member)?
            .into_active_model()
            .insert(conn)
            .await
            .map_err(PostgresError::internal)?;
    }

    let before_invites = before
        .invitations
        .iter()
        .map(|invite| (invite.id.as_str(), invite))
        .collect::<BTreeMap<_, _>>();
    let after_invites = after
        .invitations
        .iter()
        .map(|invite| (invite.id.as_str(), invite))
        .collect::<BTreeMap<_, _>>();
    for invite_id in before_invites.keys() {
        if !after_invites.contains_key(invite_id) {
            entities::repository_invite::Entity::delete_by_id((*invite_id).to_string())
                .exec(conn)
                .await
                .map_err(PostgresError::internal)?;
        }
    }
    for (invite_id, invite) in after_invites {
        let Some(old) = before_invites.get(invite_id) else {
            insert_repository_invite(conn, invite).await?;
            continue;
        };
        if *old == invite {
            continue;
        }
        entities::repository_invite::Model::from_domain(invite)?
            .into_active_model()
            .reset_all()
            .update(conn)
            .await
            .map_err(PostgresError::internal)?;
        let new_links = invite
            .link_hashes
            .iter()
            .filter(|hash| !old.link_hashes.contains(hash))
            .cloned()
            .collect::<Vec<_>>();
        insert_repository_invite_links(conn, &invite.id, &new_links).await?;
    }
    Ok(())
}

async fn insert_repository_invite<C>(
    conn: &C,
    invite: &RepositoryInvite,
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    entities::repository_invite::Model::from_domain(invite)?
        .into_active_model()
        .insert(conn)
        .await
        .map_err(PostgresError::internal)?;
    insert_repository_invite_links(conn, &invite.id, &invite.link_hashes).await
}

async fn insert_repository_invite_links<C>(
    conn: &C,
    invite_id: &str,
    link_hashes: &[String],
) -> Result<(), PostgresError>
where
    C: ConnectionTrait,
{
    if link_hashes.is_empty() {
        return Ok(());
    }
    entities::repository_invite_link::Entity::insert_many(link_hashes.iter().map(|hash| {
        entities::repository_invite_link::Model {
            token_hash: hash.clone(),
            invite_id: invite_id.to_string(),
        }
        .into_active_model()
    }))
    .exec(conn)
    .await
    .map_err(PostgresError::internal)?;
    Ok(())
}
