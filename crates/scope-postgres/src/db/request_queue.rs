use super::{RequestListRow, RequestStore, auth::load_users_by_ids, entities};
use crate::error::PostgresError;
use scope_domain::{
    account::UserAccount,
    repository::access::RepositoryAccess,
    requests::{
        REQUEST_LIST_MAX_PAGE_SIZE, RequestActorRole, RequestAttention, RequestAttentionReason,
        RequestAttentionState, RequestAudience, RequestClaim, RequestQueueClassification,
        RequestQueueFacts, RequestQueueSection, RequestState, classify_request_queue_item,
    },
};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestQueueCursor {
    pub updated_at_unix: u64,
    pub request_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestQueuePageQuery<'a> {
    pub repo_id: &'a str,
    pub section: RequestQueueSection,
    pub viewer_user_id: Option<&'a str>,
    pub access: RepositoryAccess,
    pub search: Option<&'a str>,
    pub after: Option<&'a RequestQueueCursor>,
    pub limit: u64,
    pub now_unix: u64,
}

#[derive(Clone, Debug)]
pub struct RequestQueueRow {
    pub request: RequestListRow,
    pub attention: RequestQueueClassification,
    pub claim: Option<RequestClaim>,
    pub cursor: RequestQueueCursor,
}

#[derive(Clone, Debug)]
pub struct RequestQueuePage {
    pub rows: Vec<RequestQueueRow>,
    pub users: BTreeMap<String, UserAccount>,
    pub next_attention_at_unix: Option<u64>,
}

#[derive(Debug, FromQueryResult)]
struct QueueModel {
    id: String,
    name: String,
    title: String,
    author_user_id: String,
    author_role: String,
    audience: String,
    head_oid: String,
    submitted_at_unix: Option<i64>,
    closed_at_unix: Option<i64>,
    merged_at_unix: Option<i64>,
    updated_at_unix: i64,
    activity_version: i64,
    has_git_snapshot: bool,
    viewer_is_invitee: bool,
    attention_state: Option<String>,
    attention_reason: Option<String>,
    through_activity_version: Option<i64>,
    snoozed_until_unix: Option<i64>,
    attention_updated_at_unix: Option<i64>,
    claimer_user_id: Option<String>,
    claimed_at_unix: Option<i64>,
    claim_updated_at_unix: Option<i64>,
}

impl RequestStore {
    pub async fn request_queue_page(
        &self,
        input: RequestQueuePageQuery<'_>,
    ) -> Result<RequestQueuePage, PostgresError> {
        if !input.access.is_maintainer() && input.section == RequestQueueSection::Unclaimed {
            return Ok(RequestQueuePage {
                rows: Vec::new(),
                users: BTreeMap::new(),
                next_attention_at_unix: None,
            });
        }
        let viewer = input.viewer_user_id.map(str::to_string);
        let search = input.search.map(escaped_search_pattern);
        let after_time = input
            .after
            .map(|cursor| entities::u64_to_i64(cursor.updated_at_unix, "queue cursor time"))
            .transpose()?;
        let after_id = input.after.map(|cursor| cursor.request_id.clone());
        let limit = input.limit.min((REQUEST_LIST_MAX_PAGE_SIZE + 1) as u64);
        let rows = QueueModel::find_by_statement(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            QUEUE_SQL,
            [
                input.repo_id.into(),
                viewer.clone().into(),
                input.access.is_maintainer().into(),
                (!input.access.is_maintainer()).into(),
                search.into(),
                section_name(input.section).into(),
                entities::u64_to_i64(input.now_unix, "queue time")?.into(),
                after_time.into(),
                after_id.into(),
                i64::try_from(limit)
                    .map_err(PostgresError::internal)?
                    .into(),
            ],
        ))
        .all(self.db.as_ref())
        .await
        .map_err(PostgresError::internal)?;

        let mut user_ids = Vec::with_capacity(rows.len() * 2);
        let mut projected = Vec::with_capacity(rows.len());
        for row in rows {
            user_ids.push(row.author_user_id.clone());
            if let Some(claimer) = &row.claimer_user_id {
                user_ids.push(claimer.clone());
            }
            projected.push(row.try_into_queue_row(&input)?);
        }
        let users = load_users_by_ids(self.db.as_ref(), user_ids).await?;
        let next_attention_at_unix = if input.access.is_maintainer() {
            next_snooze_expiry(
                self.db.as_ref(),
                input.repo_id,
                input.viewer_user_id,
                input.now_unix,
            )
            .await?
        } else {
            None
        };
        Ok(RequestQueuePage {
            rows: projected,
            users,
            next_attention_at_unix,
        })
    }
}

impl QueueModel {
    fn try_into_queue_row(
        self,
        input: &RequestQueuePageQuery<'_>,
    ) -> Result<RequestQueueRow, PostgresError> {
        let state = request_state(
            self.submitted_at_unix,
            self.closed_at_unix,
            self.merged_at_unix,
        );
        let activity_version =
            entities::i64_to_u64(self.activity_version, "request activity version")?;
        let attention = self.attention(input.viewer_user_id)?;
        let claim = self.claim()?;
        let classification = classify_request_queue_item(RequestQueueFacts {
            request_state: state,
            request_activity_version: activity_version,
            request_author_user_id: &self.author_user_id,
            viewer_user_id: input.viewer_user_id,
            viewer_is_maintainer: input.access.is_maintainer(),
            viewer_is_invitee: self.viewer_is_invitee,
            attention: attention.as_ref(),
            claim: claim.as_ref(),
            now_unix: input.now_unix,
        });
        if classification.section != input.section {
            return Err(PostgresError::internal_message(
                "request queue SQL and domain classification disagree",
            ));
        }
        let updated_at_unix = entities::i64_to_u64(self.updated_at_unix, "request update time")?;
        Ok(RequestQueueRow {
            cursor: RequestQueueCursor {
                updated_at_unix,
                request_id: self.id.clone(),
            },
            request: RequestListRow {
                id: self.id,
                name: self.name,
                title: self.title,
                author_user_id: self.author_user_id,
                author_role: entities::decode_enum::<RequestActorRole>(self.author_role)?,
                audience: entities::decode_enum::<RequestAudience>(self.audience)?,
                head_oid: self.head_oid,
                state,
                submitted_at_unix: optional_u64(self.submitted_at_unix, "request submission time")?,
                closed_at_unix: optional_u64(self.closed_at_unix, "request close time")?,
                merged_at_unix: optional_u64(self.merged_at_unix, "request merge time")?,
                updated_at_unix,
                activity_version,
                has_git_snapshot: self.has_git_snapshot,
            },
            attention: classification,
            claim,
        })
    }

    fn attention(
        &self,
        viewer_user_id: Option<&str>,
    ) -> Result<Option<RequestAttention>, PostgresError> {
        let Some(state) = self.attention_state.clone() else {
            return Ok(None);
        };
        let user_id = viewer_user_id.ok_or_else(|| {
            PostgresError::internal_message("attention row is missing its viewer")
        })?;
        Ok(Some(RequestAttention {
            request_id: self.id.clone(),
            user_id: user_id.to_string(),
            state: entities::decode_enum::<RequestAttentionState>(state)?,
            reason: entities::decode_enum::<RequestAttentionReason>(
                self.attention_reason.clone().ok_or_else(|| {
                    PostgresError::internal_message("attention row is missing its reason")
                })?,
            )?,
            through_activity_version: entities::i64_to_u64(
                self.through_activity_version.ok_or_else(|| {
                    PostgresError::internal_message("attention row is missing its checkpoint")
                })?,
                "request attention position",
            )?,
            snoozed_until_unix: optional_u64(self.snoozed_until_unix, "request snooze time")?,
            updated_at_unix: entities::i64_to_u64(
                self.attention_updated_at_unix.ok_or_else(|| {
                    PostgresError::internal_message("attention row is missing its update time")
                })?,
                "request attention time",
            )?,
        }))
    }

    fn claim(&self) -> Result<Option<RequestClaim>, PostgresError> {
        let Some(claimer_user_id) = self.claimer_user_id.clone() else {
            return Ok(None);
        };
        Ok(Some(RequestClaim {
            request_id: self.id.clone(),
            claimer_user_id,
            claimed_at_unix: entities::i64_to_u64(
                self.claimed_at_unix.ok_or_else(|| {
                    PostgresError::internal_message("claim row is missing its claim time")
                })?,
                "request claim time",
            )?,
            updated_at_unix: entities::i64_to_u64(
                self.claim_updated_at_unix.ok_or_else(|| {
                    PostgresError::internal_message("claim row is missing its update time")
                })?,
                "request claim update time",
            )?,
        }))
    }
}

fn request_state(
    submitted_at_unix: Option<i64>,
    closed_at_unix: Option<i64>,
    merged_at_unix: Option<i64>,
) -> RequestState {
    if merged_at_unix.is_some() {
        RequestState::Merged
    } else if closed_at_unix.is_some() {
        RequestState::Closed
    } else if submitted_at_unix.is_some() {
        RequestState::Open
    } else {
        RequestState::Draft
    }
}

fn optional_u64(value: Option<i64>, field: &str) -> Result<Option<u64>, PostgresError> {
    value
        .map(|value| entities::i64_to_u64(value, field))
        .transpose()
}

fn section_name(section: RequestQueueSection) -> &'static str {
    match section {
        RequestQueueSection::Active => "active",
        RequestQueueSection::Unclaimed => "unclaimed",
        RequestQueueSection::SetAside => "set_aside",
    }
}

fn escaped_search_pattern(value: &str) -> String {
    format!(
        "%{}%",
        value
            .replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    )
}

#[derive(Debug, FromQueryResult)]
struct NextAttention {
    next_attention_at_unix: Option<i64>,
}

async fn next_snooze_expiry<C: ConnectionTrait>(
    conn: &C,
    repo_id: &str,
    viewer_user_id: Option<&str>,
    now_unix: u64,
) -> Result<Option<u64>, PostgresError> {
    let Some(viewer_user_id) = viewer_user_id else {
        return Ok(None);
    };
    let result = NextAttention::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT MIN(a.snoozed_until_unix) AS next_attention_at_unix
         FROM scope_request_attention_states a
         JOIN scope_requests r ON r.id = a.request_id
         WHERE r.repo_id = $1 AND a.user_id = $2 AND a.state = 'snoozed'
           AND a.snoozed_until_unix > $3
           AND r.closed_at_unix IS NULL AND r.merged_at_unix IS NULL",
        [
            repo_id.into(),
            viewer_user_id.into(),
            entities::u64_to_i64(now_unix, "queue time")?.into(),
        ],
    ))
    .one(conn)
    .await
    .map_err(PostgresError::internal)?
    .ok_or_else(|| PostgresError::internal_message("snooze expiry query returned no row"))?;
    optional_u64(result.next_attention_at_unix, "request snooze time")
}

const QUEUE_SQL: &str = r#"
WITH facts AS (
    SELECT r.id, r.name, r.title, r.author_user_id, r.author_role, r.audience,
        r.head_oid, r.submitted_at_unix, r.closed_at_unix, r.merged_at_unix,
        r.updated_at_unix, r.activity_version, r.git_snapshot IS NOT NULL AS has_git_snapshot,
        EXISTS (
            SELECT 1 FROM scope_request_invitees i
            WHERE i.request_id = r.id AND i.user_id = $2
        ) AS viewer_is_invitee,
        a.state AS attention_state, a.reason AS attention_reason,
        a.through_activity_version, a.snoozed_until_unix,
        a.updated_at_unix AS attention_updated_at_unix,
        c.claimer_user_id, c.claimed_at_unix,
        c.updated_at_unix AS claim_updated_at_unix
    FROM scope_requests r
    LEFT JOIN scope_request_attention_states a
        ON a.request_id = r.id AND a.user_id = $2
    LEFT JOIN scope_request_claims c ON c.request_id = r.id
    WHERE r.repo_id = $1
      AND (
        NOT $4 OR r.audience = 'Public' OR
        ($2 IS NOT NULL AND (
            r.author_user_id = $2 OR
            EXISTS (
                SELECT 1 FROM scope_request_invitees i
                WHERE i.request_id = r.id AND i.user_id = $2
            )
        ))
      )
      AND (
        r.submitted_at_unix IS NOT NULL OR
        ($2 IS NOT NULL AND r.author_user_id = $2) OR
        EXISTS (
            SELECT 1 FROM scope_request_invitees i
            WHERE i.request_id = r.id AND i.user_id = $2
        )
      )
      AND ($5::text IS NULL OR r.title ILIKE $5 ESCAPE '\' OR r.description_markdown ILIKE $5 ESCAPE '\')
), classified AS (
    SELECT facts.*, CASE
        WHEN closed_at_unix IS NOT NULL OR merged_at_unix IS NOT NULL THEN 'set_aside'
        WHEN $3 AND attention_state IN ('waiting', 'settled') THEN 'set_aside'
        WHEN $3 AND attention_state = 'snoozed' AND snoozed_until_unix > $7 THEN 'set_aside'
        WHEN $3 AND claimer_user_id IS NOT NULL AND claimer_user_id <> $2 THEN 'set_aside'
        WHEN NOT $3 THEN 'active'
        WHEN claimer_user_id = $2 OR attention_state = 'active' OR
             (attention_state = 'snoozed' AND snoozed_until_unix <= $7) OR
             author_user_id = $2 OR viewer_is_invitee THEN 'active'
        ELSE 'unclaimed'
    END AS queue_section
    FROM facts
)
SELECT id, name, title, author_user_id, author_role, audience, head_oid,
    submitted_at_unix, closed_at_unix, merged_at_unix, updated_at_unix,
    activity_version, has_git_snapshot, viewer_is_invitee, attention_state,
    attention_reason, through_activity_version, snoozed_until_unix,
    attention_updated_at_unix, claimer_user_id, claimed_at_unix, claim_updated_at_unix
FROM classified
WHERE queue_section = $6
  AND ($8::bigint IS NULL OR updated_at_unix < $8 OR (updated_at_unix = $8 AND id > $9))
ORDER BY updated_at_unix DESC, id ASC
LIMIT $10
"#;
