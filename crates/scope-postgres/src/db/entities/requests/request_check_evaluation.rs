use super::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_request_check_evaluations")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub request_id: String,
    pub revision_id: String,
    pub base_version: i64,
    pub review_base_oid: String,
    pub github_repository_id: Option<i64>,
    pub head_oid: String,
    pub tested_oid: String,
    pub changes_github_workflows: Option<bool>,
    pub state: String,
    pub message: Option<String>,
    pub checks: Json,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
    pub check_private_main_oid: Option<String>,
    pub check_public_base_oid: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}
impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub fn from_domain(value: &RequestCheckEvaluation) -> Result<Self, PostgresError> {
        Ok(Self {
            id: value.id.clone(),
            revision_id: value.revision_id.clone(),
            base_version: u64_to_i64(value.base_version, "request check base version")?,
            review_base_oid: value.review_base_oid.clone(),
            github_repository_id: value
                .github_repository_id
                .map(|id| u64_to_i64(id, "GitHub repository id"))
                .transpose()?,
            request_id: value.request_id.clone(),
            head_oid: value.head_oid.clone(),
            tested_oid: value.tested_oid.clone(),
            changes_github_workflows: value.changes_github_workflows,
            state: encode_enum(value.state)?,
            message: value.message.clone(),
            checks: encode_json(&value.checks)?,
            created_at_unix: u64_to_i64(value.created_at_unix, "request check creation time")?,
            updated_at_unix: u64_to_i64(value.updated_at_unix, "request check update time")?,
            check_private_main_oid: value
                .check_commit_base
                .as_ref()
                .map(|base| base.canonical_main_oid.clone()),
            check_public_base_oid: value
                .check_commit_base
                .as_ref()
                .map(|base| base.view_base_oid.clone()),
        })
    }

    pub fn try_into_domain(self) -> Result<RequestCheckEvaluation, PostgresError> {
        Ok(RequestCheckEvaluation {
            id: self.id,
            revision_id: self.revision_id,
            base_version: i64_to_u64(self.base_version, "request check base version")?,
            review_base_oid: self.review_base_oid,
            github_repository_id: self
                .github_repository_id
                .map(|id| i64_to_u64(id, "GitHub repository id"))
                .transpose()?,
            request_id: self.request_id,
            head_oid: self.head_oid,
            tested_oid: self.tested_oid,
            changes_github_workflows: self.changes_github_workflows,
            check_commit_base: match (self.check_private_main_oid, self.check_public_base_oid) {
                (Some(canonical_main_oid), Some(view_base_oid)) => {
                    Some(CheckCommitBase::new(canonical_main_oid, view_base_oid)?)
                }
                (None, None) => None,
                _ => {
                    return Err(PostgresError::internal_message(
                        "request check commit names only one of its bases",
                    ));
                }
            },
            state: decode_enum(self.state)?,
            message: self.message,
            checks: decode_json(self.checks)?,
            created_at_unix: i64_to_u64(self.created_at_unix, "request check creation time")?,
            updated_at_unix: i64_to_u64(self.updated_at_unix, "request check update time")?,
        })
    }
}
