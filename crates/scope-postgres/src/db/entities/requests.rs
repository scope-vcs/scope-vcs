use super::*;
use scope_domain::requests::{
    CheckCommitBase, Request, RequestActorRole, RequestAttention, RequestAttentionReason,
    RequestAttentionState, RequestBase, RequestCheckEvaluation, RequestClaim, RequestDiffSide,
    RequestDiscussion, RequestDiscussionAnchor, RequestDiscussionLines, RequestDiscussionReadState,
    RequestDiscussionReply, RequestDiscussionStatus, RequestEvent, RequestEventKind,
    RequestEventPayload, RequestInvitee, RequestLineRange, RequestRating, RequestRevision,
    RequestStackStatus,
};
use scope_domain::views::ViewId;
use scope_domain::{content::SourceBlob, policy::ScopePath};

pub mod request;

pub mod request_claim {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_claims")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub request_id: String,
        pub claimer_user_id: String,
        pub claimed_at_unix: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestClaim) -> Result<Self, PostgresError> {
            Ok(Self {
                request_id: value.request_id.clone(),
                claimer_user_id: value.claimer_user_id.clone(),
                claimed_at_unix: u64_to_i64(value.claimed_at_unix, "request claim time")?,
                updated_at_unix: u64_to_i64(value.updated_at_unix, "request claim update time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestClaim, PostgresError> {
            Ok(RequestClaim {
                request_id: self.request_id,
                claimer_user_id: self.claimer_user_id,
                claimed_at_unix: i64_to_u64(self.claimed_at_unix, "request claim time")?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "request claim update time")?,
            })
        }
    }
}

pub mod request_attention_state {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_attention_states")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub request_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub state: String,
        pub reason: String,
        pub through_activity_version: i64,
        pub snoozed_until_unix: Option<i64>,
        pub updated_at_unix: i64,
        pub revision: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}
    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestAttention) -> Result<Self, PostgresError> {
            Ok(Self {
                request_id: value.request_id.clone(),
                user_id: value.user_id.clone(),
                state: encode_enum(value.state)?,
                reason: encode_enum(value.reason)?,
                through_activity_version: u64_to_i64(
                    value.through_activity_version,
                    "request attention position",
                )?,
                snoozed_until_unix: value
                    .snoozed_until_unix
                    .map(|value| u64_to_i64(value, "request snooze time"))
                    .transpose()?,
                updated_at_unix: u64_to_i64(value.updated_at_unix, "request attention time")?,
                revision: u64_to_i64(value.revision, "request attention revision")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestAttention, PostgresError> {
            Ok(RequestAttention {
                request_id: self.request_id,
                user_id: self.user_id,
                state: decode_enum::<RequestAttentionState>(self.state)?,
                reason: decode_enum::<RequestAttentionReason>(self.reason)?,
                through_activity_version: i64_to_u64(
                    self.through_activity_version,
                    "request attention position",
                )?,
                snoozed_until_unix: self
                    .snoozed_until_unix
                    .map(|value| i64_to_u64(value, "request snooze time"))
                    .transpose()?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "request attention time")?,
                revision: i64_to_u64(self.revision, "request attention revision")?,
            })
        }
    }
}

pub mod request_rating {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_ratings")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub request_id: String,
        pub rater_user_id: String,
        pub subject_user_id: String,
        pub score: i32,
        pub reason: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(rating: &RequestRating) -> Result<Self, PostgresError> {
            Ok(Self {
                id: rating.id.clone(),
                request_id: rating.request_id.clone(),
                rater_user_id: rating.rater_user_id.clone(),
                subject_user_id: rating.subject_user_id.clone(),
                score: i32::from(rating.score),
                reason: rating.reason.clone(),
                created_at_unix: u64_to_i64(
                    rating.created_at_unix,
                    "request rating creation time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestRating, PostgresError> {
            Ok(RequestRating {
                id: self.id,
                request_id: self.request_id,
                rater_user_id: self.rater_user_id,
                subject_user_id: self.subject_user_id,
                score: self.score.try_into().map_err(PostgresError::internal)?,
                reason: self.reason,
                created_at_unix: i64_to_u64(self.created_at_unix, "request rating creation time")?,
            })
        }
    }
}

pub mod request_invitee {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_invitees")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub request_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub invited_by_user_id: Option<String>,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(invitee: &RequestInvitee) -> Result<Self, PostgresError> {
            Ok(Self {
                request_id: invitee.request_id.clone(),
                user_id: invitee.user_id.clone(),
                invited_by_user_id: invitee.invited_by_user_id.clone(),
                created_at_unix: u64_to_i64(
                    invitee.created_at_unix,
                    "request invitee creation time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestInvitee, PostgresError> {
            Ok(RequestInvitee {
                request_id: self.request_id,
                user_id: self.user_id,
                invited_by_user_id: self.invited_by_user_id,
                created_at_unix: i64_to_u64(self.created_at_unix, "request invitee creation time")?,
            })
        }
    }
}

pub mod request_revision;

pub mod request_event {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_events")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub request_id: String,
        pub actor_user_id: Option<String>,
        pub kind: String,
        pub position: i64,
        pub payload: Json,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(event: &RequestEvent) -> Result<Self, PostgresError> {
            Ok(Self {
                id: event.id.clone(),
                request_id: event.request_id.clone(),
                actor_user_id: event.actor_user_id.clone(),
                kind: encode_enum(event.kind)?,
                position: u64_to_i64(event.position, "request event position")?,
                payload: encode_json(&event.payload)?,
                created_at_unix: u64_to_i64(event.created_at_unix, "request event creation time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestEvent, PostgresError> {
            Ok(RequestEvent {
                id: self.id,
                request_id: self.request_id,
                actor_user_id: self.actor_user_id,
                kind: decode_enum::<RequestEventKind>(self.kind)?,
                position: i64_to_u64(self.position, "request event position")?,
                payload: decode_json::<RequestEventPayload>(self.payload)?,
                created_at_unix: i64_to_u64(self.created_at_unix, "request event creation time")?,
            })
        }
    }
}

pub mod request_discussion {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussions")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub request_id: String,
        pub opened_position: i64,
        pub last_activity_position: i64,
        pub author_user_id: Option<String>,
        pub body_markdown: String,
        pub revision_id: Option<String>,
        pub commit_oid: Option<String>,
        pub path: Option<String>,
        pub line_side: Option<String>,
        pub start_line: Option<i32>,
        pub end_line: Option<i32>,
        pub status: String,
        pub client_discussion_id: String,
        pub created_at_unix: i64,
        pub resolved_at_unix: Option<i64>,
        pub resolved_by_user_id: Option<String>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussion) -> Result<Self, PostgresError> {
            let lines = value.anchor.as_ref().and_then(|anchor| anchor.lines);
            Ok(Self {
                id: value.id.clone(),
                request_id: value.request_id.clone(),
                opened_position: u64_to_i64(value.opened_position, "discussion opened position")?,
                last_activity_position: u64_to_i64(
                    value.last_activity_position,
                    "discussion last activity position",
                )?,
                author_user_id: value.author_user_id.clone(),
                body_markdown: value.body_markdown.clone(),
                revision_id: value
                    .anchor
                    .as_ref()
                    .map(|anchor| anchor.revision_id.clone()),
                commit_oid: value
                    .anchor
                    .as_ref()
                    .and_then(|anchor| anchor.commit_oid.clone()),
                path: value
                    .anchor
                    .as_ref()
                    .and_then(|anchor| anchor.path.as_ref())
                    .map(|path| path.as_str().to_string()),
                line_side: lines.map(|lines| encode_enum(lines.side)).transpose()?,
                start_line: lines
                    .map(|lines| u32_to_i32(lines.range.start_line(), "discussion start line"))
                    .transpose()?,
                end_line: lines
                    .map(|lines| u32_to_i32(lines.range.end_line(), "discussion end line"))
                    .transpose()?,
                status: encode_enum(value.status)?,
                client_discussion_id: value.client_discussion_id.clone(),
                created_at_unix: u64_to_i64(value.created_at_unix, "discussion creation time")?,
                resolved_at_unix: optional_u64_to_i64(
                    value.resolved_at_unix,
                    "discussion resolution time",
                )?,
                resolved_by_user_id: value.resolved_by_user_id.clone(),
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussion, PostgresError> {
            let anchor = self
                .revision_id
                .map(|revision_id| -> Result<_, PostgresError> {
                    Ok(RequestDiscussionAnchor {
                        revision_id,
                        commit_oid: self.commit_oid,
                        path: self
                            .path
                            .map(ScopePath::parse)
                            .transpose()
                            .map_err(PostgresError::internal)?,
                        lines: discussion_lines(self.line_side, self.start_line, self.end_line)?,
                    })
                })
                .transpose()?;
            Ok(RequestDiscussion {
                id: self.id,
                request_id: self.request_id,
                opened_position: i64_to_u64(self.opened_position, "discussion opened position")?,
                last_activity_position: i64_to_u64(
                    self.last_activity_position,
                    "discussion last activity position",
                )?,
                author_user_id: self.author_user_id,
                body_markdown: self.body_markdown,
                anchor,
                status: decode_enum::<RequestDiscussionStatus>(self.status)?,
                client_discussion_id: self.client_discussion_id,
                created_at_unix: i64_to_u64(self.created_at_unix, "discussion creation time")?,
                resolved_at_unix: optional_i64_to_u64(
                    self.resolved_at_unix,
                    "discussion resolution time",
                )?,
                resolved_by_user_id: self.resolved_by_user_id,
            })
        }
    }

    fn discussion_lines(
        side: Option<String>,
        start_line: Option<i32>,
        end_line: Option<i32>,
    ) -> Result<Option<RequestDiscussionLines>, PostgresError> {
        match (side, start_line, end_line) {
            (None, None, None) => Ok(None),
            (Some(side), Some(start_line), Some(end_line)) => Ok(Some(RequestDiscussionLines {
                side: decode_enum::<RequestDiffSide>(side)?,
                range: RequestLineRange::new(
                    i32_to_u32(start_line, "discussion start line")?,
                    i32_to_u32(end_line, "discussion end line")?,
                )
                .map_err(PostgresError::internal)?,
            })),
            _ => Err(PostgresError::internal_message(
                "request discussion line anchor is incomplete",
            )),
        }
    }
}

pub mod request_discussion_reply {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussion_replies")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        pub discussion_id: String,
        pub position: i64,
        pub author_user_id: Option<String>,
        pub body_markdown: String,
        pub reply_to_reply_id: Option<String>,
        pub client_reply_id: String,
        pub created_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussionReply) -> Result<Self, PostgresError> {
            Ok(Self {
                id: value.id.clone(),
                discussion_id: value.discussion_id.clone(),
                position: u64_to_i64(value.position, "discussion reply position")?,
                author_user_id: value.author_user_id.clone(),
                body_markdown: value.body_markdown.clone(),
                reply_to_reply_id: value.reply_to_reply_id.clone(),
                client_reply_id: value.client_reply_id.clone(),
                created_at_unix: u64_to_i64(
                    value.created_at_unix,
                    "discussion reply creation time",
                )?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussionReply, PostgresError> {
            Ok(RequestDiscussionReply {
                id: self.id,
                discussion_id: self.discussion_id,
                position: i64_to_u64(self.position, "discussion reply position")?,
                author_user_id: self.author_user_id,
                body_markdown: self.body_markdown,
                reply_to_reply_id: self.reply_to_reply_id,
                client_reply_id: self.client_reply_id,
                created_at_unix: i64_to_u64(
                    self.created_at_unix,
                    "discussion reply creation time",
                )?,
            })
        }
    }
}

pub mod request_discussion_read_state {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(table_name = "scope_request_discussion_read_states")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub discussion_id: String,
        #[sea_orm(primary_key, auto_increment = false)]
        pub user_id: String,
        pub read_through_position: i64,
        pub updated_at_unix: i64,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}

    impl Model {
        pub fn from_domain(value: &RequestDiscussionReadState) -> Result<Self, PostgresError> {
            Ok(Self {
                discussion_id: value.discussion_id.clone(),
                user_id: value.user_id.clone(),
                read_through_position: u64_to_i64(
                    value.read_through_position,
                    "discussion read position",
                )?,
                updated_at_unix: u64_to_i64(value.updated_at_unix, "discussion read time")?,
            })
        }

        pub fn try_into_domain(self) -> Result<RequestDiscussionReadState, PostgresError> {
            Ok(RequestDiscussionReadState {
                discussion_id: self.discussion_id,
                user_id: self.user_id,
                read_through_position: i64_to_u64(
                    self.read_through_position,
                    "discussion read position",
                )?,
                updated_at_unix: i64_to_u64(self.updated_at_unix, "discussion read time")?,
            })
        }
    }
}

pub mod request_check_evaluation;

pub(crate) mod base;
pub mod revision_context_receipt;
