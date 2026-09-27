use crate::db::{entities, generated_ids::test_generated_id, requests::tests::postgres_store};
use scope_domain::{
    repo_collaboration::{REPOSITORY_INVITE_RETENTION_SECS, REPOSITORY_INVITE_TTL_SECS},
    repository::collaboration::{RepositoryInvite, RepositoryMember, RepositoryMemberPermissions},
};
use sea_orm::{ActiveModelTrait, EntityTrait, IntoActiveModel};

const REPO_ID: &str = "owner/repo";
const NOW: u64 = 100 * 24 * 60 * 60;
const CUTOFF: u64 = NOW - REPOSITORY_INVITE_RETENTION_SECS;

enum Ended {
    Accepted(u64),
    Revoked(u64),
    Expired(u64),
    Pending,
}

fn invite(id: &str, ended: Ended) -> RepositoryInvite {
    let expires_at_unix = match ended {
        Ended::Expired(at) => at,
        _ => NOW + 1,
    };
    let created_at_unix = expires_at_unix - REPOSITORY_INVITE_TTL_SECS;
    let email = format!("{id}@example.com");
    RepositoryInvite {
        id: id.to_string(),
        repo_id: REPO_ID.to_string(),
        invited_email: email.clone(),
        invited_email_normalized: email,
        permissions: RepositoryMemberPermissions::default(),
        invited_by_user_id: "user_owner".to_string(),
        link_hashes: vec![format!("sha256:{id}")],
        created_at_unix,
        updated_at_unix: created_at_unix,
        expires_at_unix,
        accepted_by_user_id: matches!(ended, Ended::Accepted(_)).then(|| "user_public".into()),
        accepted_at_unix: match ended {
            Ended::Accepted(at) => Some(at),
            _ => None,
        },
        revoked_at_unix: match ended {
            Ended::Revoked(at) => Some(at),
            _ => None,
        },
    }
}

#[tokio::test]
async fn retention_deletes_invites_over_for_thirty_days_with_their_links_and_emails() {
    let store = postgres_store();
    let repositories = store.repositories();
    let invites = [
        invite("accepted_old", Ended::Accepted(CUTOFF)),
        invite("revoked_old", Ended::Revoked(CUTOFF - 1)),
        invite("expired_old", Ended::Expired(CUTOFF)),
        invite("accepted_recent", Ended::Accepted(CUTOFF + 1)),
        invite("revoked_recent", Ended::Revoked(NOW - 1)),
        invite("expired_recent", Ended::Expired(CUTOFF + 1)),
        invite("pending", Ended::Pending),
    ];
    let member = RepositoryMember {
        repo_id: REPO_ID.to_string(),
        user_id: "user_public".to_string(),
        permissions: RepositoryMemberPermissions::default(),
        created_at_unix: CUTOFF,
        updated_at_unix: CUTOFF,
    };
    let seeded = invites.to_vec();
    let seeded_member = member.clone();
    repositories
        .mutate_repository_for_tests(REPO_ID, move |repo| {
            repo.invitations = seeded;
            repo.members = vec![seeded_member];
        })
        .await
        .unwrap();
    for invite in &invites {
        entities::repository_invite_email::Model {
            id: format!("email_{}", invite.id),
            invite_id: Some(invite.id.clone()),
            requested_by_user_id: "user_owner".to_string(),
            state: "Sent".to_string(),
            attempts: 1,
            next_attempt_at_unix: invite.created_at_unix as i64,
            claim_token: None,
            claim_expires_at_unix: None,
            provider_message_id: None,
            last_error: None,
            created_at_unix: invite.created_at_unix as i64,
            updated_at_unix: invite.created_at_unix as i64,
        }
        .into_active_model()
        .insert(store.db.as_ref())
        .await
        .unwrap();
    }
    let version = repositories
        .repository_for_tests(REPO_ID)
        .await
        .unwrap()
        .unwrap()
        .record
        .change_version;

    assert_eq!(
        repositories
            .repositories_with_prunable_invites(NOW, 10)
            .await
            .unwrap(),
        [REPO_ID]
    );
    let mutation = repositories
        .prune_ended_repository_invites(REPO_ID, NOW, &test_generated_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(mutation.value, 3);
    assert_eq!(mutation.change_version, version + 1);

    let kept = [
        "accepted_recent",
        "expired_recent",
        "pending",
        "revoked_recent",
    ];
    let repo = repositories
        .repository_for_tests(REPO_ID)
        .await
        .unwrap()
        .unwrap();
    let mut invite_ids = repo
        .invitations
        .iter()
        .map(|invite| invite.id.as_str())
        .collect::<Vec<_>>();
    invite_ids.sort_unstable();
    assert_eq!(invite_ids, kept);
    assert_eq!(repo.members, [member]);

    let mut link_owners = entities::repository_invite_link::Entity::find()
        .all(store.db.as_ref())
        .await
        .unwrap()
        .into_iter()
        .map(|link| link.invite_id)
        .collect::<Vec<_>>();
    link_owners.sort_unstable();
    assert_eq!(link_owners, kept);
    let mut email_owners = entities::repository_invite_email::Entity::find()
        .all(store.db.as_ref())
        .await
        .unwrap()
        .into_iter()
        .map(|email| email.invite_id)
        .collect::<Vec<_>>();
    email_owners.sort_unstable();
    assert_eq!(email_owners, kept.map(|id| Some(id.to_string())));

    assert!(
        repositories
            .repositories_with_prunable_invites(NOW, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        repositories
            .prune_ended_repository_invites(REPO_ID, NOW, &test_generated_id)
            .await
            .unwrap()
            .is_none()
    );
}
