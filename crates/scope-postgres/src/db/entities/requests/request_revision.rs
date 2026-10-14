use super::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_request_revisions")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub request_id: String,
    pub position: i64,
    pub actor_user_id: Option<String>,
    pub old_head_oid: String,
    pub new_head_oid: String,
    pub base_kind: String,
    pub review_base_oid: String,
    pub base_version: i64,
    pub parent_request_id: Option<String>,
    pub parent_revision_id: Option<String>,
    pub parent_head_oid: Option<String>,
    pub main_ancestor_oid: String,
    pub rewrote_history: bool,
    pub git_snapshot: Json,
    pub created_at_unix: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub fn from_domain(value: &RequestRevision) -> Result<Self, PostgresError> {
        let base = base::BaseColumns::from_domain(&value.base);
        Ok(Self {
            id: value.id.clone(),
            request_id: value.request_id.clone(),
            position: u64_to_i64(value.position, "request revision position")?,
            actor_user_id: value.actor_user_id.clone(),
            old_head_oid: value.old_head_oid.clone(),
            new_head_oid: value.new_head_oid.clone(),
            base_kind: base.kind,
            review_base_oid: base.review_base_oid,
            base_version: u64_to_i64(value.base_version, "request revision base version")?,
            parent_request_id: base.parent_request_id,
            parent_revision_id: base.parent_revision_id,
            parent_head_oid: base.parent_head_oid,
            main_ancestor_oid: value.main_ancestor_oid.clone(),
            rewrote_history: value.rewrote_history,
            git_snapshot: encode_json(&value.git_snapshot)?,
            created_at_unix: u64_to_i64(value.created_at_unix, "request revision creation time")?,
        })
    }

    pub fn try_into_domain(self) -> Result<RequestRevision, PostgresError> {
        Ok(RequestRevision {
            id: self.id,
            request_id: self.request_id,
            position: i64_to_u64(self.position, "request revision position")?,
            actor_user_id: self.actor_user_id,
            old_head_oid: self.old_head_oid,
            new_head_oid: self.new_head_oid,
            base: base::BaseColumns {
                kind: self.base_kind,
                review_base_oid: self.review_base_oid,
                parent_request_id: self.parent_request_id,
                parent_revision_id: self.parent_revision_id,
                parent_head_oid: self.parent_head_oid,
            }
            .try_into_domain()?,
            base_version: i64_to_u64(self.base_version, "request revision base version")?,
            main_ancestor_oid: self.main_ancestor_oid,
            rewrote_history: self.rewrote_history,
            git_snapshot: decode_json::<SourceBlob>(self.git_snapshot)?,
            created_at_unix: i64_to_u64(self.created_at_unix, "request revision creation time")?,
        })
    }
}
