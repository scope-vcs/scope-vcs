use super::*;
use scope_domain::requests::{
    RequestAutoMergeIntent, RequestAutoMergeIntentStatus, RequestAutoMergeStopReason,
};

pub mod request_auto_merge_intent {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_auto_merge_intents")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub repo_id: String,
        pub repository_incarnation_id: String,
        pub request_id: String,
        pub revision_id: String,
        pub head_oid: String,
        pub actor_user_id: String,
        pub status: String,
        pub reason: Option<String>,
        pub created_position: i64,
        pub claim_token: Option<String>,
        pub claim_expires_at_unix: Option<i64>,
        pub attempt: i32,
        pub next_attempt_at_unix: i64,
        pub last_error: Option<String>,
        pub created_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(
            value: &RequestAutoMergeIntent,
            created_position: u64,
        ) -> Result<Self, PostgresError> {
            value.validate_facts()?;
            Ok(Self {
                id: value.id.clone(),
                repo_id: value.repo_id.clone(),
                repository_incarnation_id: value.repository_incarnation_id.clone(),
                request_id: value.request_id.clone(),
                revision_id: value.revision_id.clone(),
                head_oid: value.head_oid.clone(),
                actor_user_id: value.actor_user_id.clone(),
                status: encode_enum(value.status)?,
                reason: value.reason.map(encode_enum).transpose()?,
                created_position: u64_to_i64(created_position, "auto-merge creation position")?,
                claim_token: None,
                claim_expires_at_unix: None,
                attempt: 0,
                next_attempt_at_unix: u64_to_i64(
                    value.updated_at_unix,
                    "auto-merge next attempt time",
                )?,
                last_error: None,
                created_at_unix: u64_to_i64(value.created_at_unix, "auto-merge creation time")?,
                updated_at_unix: u64_to_i64(value.updated_at_unix, "auto-merge update time")?,
            })
        }

        pub fn try_into_domain(&self) -> Result<RequestAutoMergeIntent, PostgresError> {
            let value = RequestAutoMergeIntent {
                id: self.id.clone(),
                repo_id: self.repo_id.clone(),
                repository_incarnation_id: self.repository_incarnation_id.clone(),
                request_id: self.request_id.clone(),
                revision_id: self.revision_id.clone(),
                head_oid: self.head_oid.clone(),
                actor_user_id: self.actor_user_id.clone(),
                status: decode_enum::<RequestAutoMergeIntentStatus>(self.status.clone())?,
                reason: self
                    .reason
                    .clone()
                    .map(decode_enum::<RequestAutoMergeStopReason>)
                    .transpose()?,
                created_at_unix: i64_to_u64(self.created_at_unix, "auto-merge creation time")?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "auto-merge update time")?,
            };
            value.validate_facts()?;
            Ok(value)
        }
    }
}
