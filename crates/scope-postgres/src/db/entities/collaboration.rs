use super::*;

pub mod repository_member {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_members")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub repo_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub permissions: Json,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(member: &RepositoryMember) -> Result<Self, PostgresError> {
            Ok(Self {
                repo_id: member.repo_id.clone(),
                user_id: member.user_id.clone(),
                permissions: encode_json(&member.permissions)?,
                created_at_unix: u64_to_i64(
                    member.created_at_unix,
                    "repository member creation time",
                )?,
                updated_at_unix: u64_to_i64(
                    member.updated_at_unix,
                    "repository member update time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RepositoryMember, PostgresError> {
            Ok(RepositoryMember {
                repo_id: self.repo_id,
                user_id: self.user_id,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "repository member creation time",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "repository member update time")?,
            })
        }
    }
}
pub mod repository_invite {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_invites")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub repo_id: String,
        pub invited_email: String,
        pub invited_email_normalized: String,
        pub permissions: Json,
        pub invited_by_user_id: String,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
        pub expires_at_unix: i64,
        pub accepted_by_user_id: Option<String>,
        pub accepted_at_unix: Option<i64>,
        pub revoked_at_unix: Option<i64>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(invite: &RepositoryInvite) -> Result<Self, PostgresError> {
            Ok(Self {
                id: invite.id.clone(),
                repo_id: invite.repo_id.clone(),
                invited_email: invite.invited_email.clone(),
                invited_email_normalized: invite.invited_email_normalized.clone(),
                permissions: encode_json(&invite.permissions)?,
                invited_by_user_id: invite.invited_by_user_id.clone(),
                created_at_unix: u64_to_i64(
                    invite.created_at_unix,
                    "repository invite creation time",
                )?,
                updated_at_unix: u64_to_i64(
                    invite.updated_at_unix,
                    "repository invite update time",
                )?,
                expires_at_unix: u64_to_i64(
                    invite.expires_at_unix,
                    "repository invite expiry time",
                )?,
                accepted_by_user_id: invite.accepted_by_user_id.clone(),
                accepted_at_unix: optional_u64_to_i64(
                    invite.accepted_at_unix,
                    "repository invite acceptance time",
                )?,
                revoked_at_unix: optional_u64_to_i64(
                    invite.revoked_at_unix,
                    "repository invite revocation time",
                )?,
            })
        }

        pub fn try_into_domain(
            self,
            link_hashes: Vec<String>,
        ) -> Result<RepositoryInvite, PostgresError> {
            Ok(RepositoryInvite {
                id: self.id,
                repo_id: self.repo_id,
                invited_email: self.invited_email,
                invited_email_normalized: self.invited_email_normalized,
                permissions: decode_json::<RepositoryMemberPermissions>(self.permissions)?,
                invited_by_user_id: self.invited_by_user_id,
                link_hashes,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "repository invite creation time",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "repository invite update time")?,
                expires_at_unix: i64_to_u64(self.expires_at_unix, "repository invite expiry time")?,
                accepted_by_user_id: self.accepted_by_user_id,
                accepted_at_unix: optional_i64_to_u64(
                    self.accepted_at_unix,
                    "repository invite acceptance time",
                )?,
                revoked_at_unix: optional_i64_to_u64(
                    self.revoked_at_unix,
                    "repository invite revocation time",
                )?,
            })
        }
    }
}
pub mod repository_invite_link {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_invite_links")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub token_hash: String,
        pub invite_id: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
pub mod repository_invite_email {
    use super::*;
    use scope_domain::repo_invite_email::{RepositoryInviteEmail, RepositoryInviteEmailState};

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_repository_invite_emails")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub invite_id: Option<String>,
        pub requested_by_user_id: String,
        pub state: String,
        pub attempts: i32,
        pub next_attempt_at_unix: i64,
        pub claim_token: Option<String>,
        pub claim_expires_at_unix: Option<i64>,
        pub provider_message_id: Option<String>,
        pub last_error: Option<String>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    pub fn state_name(state: RepositoryInviteEmailState) -> &'static str {
        match state {
            RepositoryInviteEmailState::Queued => "Queued",
            RepositoryInviteEmailState::Sent => "Sent",
            RepositoryInviteEmailState::Failed => "Failed",
        }
    }

    impl Model {
        pub fn try_into_domain(self) -> Result<RepositoryInviteEmail, PostgresError> {
            let state = match self.state.as_str() {
                "Queued" => RepositoryInviteEmailState::Queued,
                "Sent" => RepositoryInviteEmailState::Sent,
                "Failed" => RepositoryInviteEmailState::Failed,
                other => {
                    return Err(PostgresError::internal_message(format!(
                        "unknown repository invite email state {other}"
                    )));
                }
            };
            Ok(RepositoryInviteEmail {
                id: self.id,
                // Only rows kept for the owner's allowance lose their invite,
                // and nothing loads those as emails.
                invite_id: self
                    .invite_id
                    .ok_or_else(|| PostgresError::internal_message("invite email has no invite"))?,
                requested_by_user_id: self.requested_by_user_id,
                state,
                attempts: u32::try_from(self.attempts).map_err(PostgresError::internal)?,
                created_at_unix: i64_to_u64(self.created_at_unix, "invite email creation time")?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "invite email update time")?,
            })
        }
    }
}
