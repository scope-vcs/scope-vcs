pub(crate) use super::base::BaseColumns;
use super::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_requests")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub author_user_id: Option<String>,
    pub author_role: String,
    pub audience: String,
    pub base_kind: String,
    pub review_base_oid: String,
    pub parent_request_id: Option<String>,
    pub parent_revision_id: Option<String>,
    pub parent_head_oid: Option<String>,
    pub base_version: i64,
    pub stack_status: String,
    pub stack_depth: i32,
    pub current_check_evaluation_id: Option<String>,
    pub main_ancestor_oid: String,
    pub head_oid: String,
    pub git_snapshot: Option<Json>,
    pub title: String,
    pub description_markdown: String,
    pub activity_version: i64,
    pub submitted_at_unix: Option<i64>,
    pub closed_at_unix: Option<i64>,
    pub closed_by_user_id: Option<String>,
    pub merged_at_unix: Option<i64>,
    pub merged_by_user_id: Option<String>,
    pub merged_head_oid: Option<String>,
    pub merged_main_oid: Option<String>,
    pub created_at_unix: i64,
    pub updated_at_unix: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}

impl Model {
    pub fn from_domain(request: &Request) -> Result<Self, PostgresError> {
        request.validate_facts()?;
        let base = base::BaseColumns::from_domain(&request.base);
        Ok(Self {
            id: request.id.clone(),
            repo_id: request.repo_id.clone(),
            name: request.name.clone(),
            author_user_id: request.author_user_id.clone(),
            author_role: encode_enum(request.author_role)?,
            audience: request.view.as_str().to_string(),
            base_kind: base.kind,
            review_base_oid: base.review_base_oid,
            parent_request_id: base.parent_request_id,
            parent_revision_id: base.parent_revision_id,
            parent_head_oid: base.parent_head_oid,
            base_version: u64_to_i64(request.base_version, "request base version")?,
            stack_status: encode_enum(request.stack_status)?,
            stack_depth: request
                .stack_depth
                .try_into()
                .map_err(PostgresError::internal)?,
            current_check_evaluation_id: None,
            main_ancestor_oid: request.main_ancestor_oid.clone(),
            head_oid: request.head_oid.clone(),
            git_snapshot: request.git_snapshot.as_ref().map(encode_json).transpose()?,
            title: request.title.clone(),
            description_markdown: request.description_markdown.clone(),
            activity_version: u64_to_i64(request.activity_version, "request activity version")?,
            submitted_at_unix: optional_u64_to_i64(
                request.submitted_at_unix,
                "request submission time",
            )?,
            closed_at_unix: optional_u64_to_i64(request.closed_at_unix, "request close time")?,
            closed_by_user_id: request.closed_by_user_id.clone(),
            merged_at_unix: optional_u64_to_i64(request.merged_at_unix, "request merge time")?,
            merged_by_user_id: request.merged_by_user_id.clone(),
            merged_head_oid: request.merged_head_oid.clone(),
            merged_main_oid: request.merged_main_oid.clone(),
            created_at_unix: u64_to_i64(request.created_at_unix, "request creation time")?,
            updated_at_unix: u64_to_i64(request.updated_at_unix, "request update time")?,
        })
    }

    pub fn try_into_domain(self) -> Result<Request, PostgresError> {
        let request = Request {
            id: self.id,
            repo_id: self.repo_id,
            name: self.name,
            author_user_id: self.author_user_id,
            author_role: decode_enum::<RequestActorRole>(self.author_role)?,
            view: ViewId::parse(&self.audience).map_err(PostgresError::internal)?,
            base: base::BaseColumns {
                kind: self.base_kind,
                review_base_oid: self.review_base_oid,
                parent_request_id: self.parent_request_id,
                parent_revision_id: self.parent_revision_id,
                parent_head_oid: self.parent_head_oid,
            }
            .try_into_domain()?,
            base_version: i64_to_u64(self.base_version, "request base version")?,
            stack_status: decode_enum::<RequestStackStatus>(self.stack_status)?,
            stack_depth: self
                .stack_depth
                .try_into()
                .map_err(PostgresError::internal)?,
            main_ancestor_oid: self.main_ancestor_oid,
            head_oid: self.head_oid,
            git_snapshot: self
                .git_snapshot
                .map(decode_json::<SourceBlob>)
                .transpose()?,
            title: self.title,
            description_markdown: self.description_markdown,
            activity_version: i64_to_u64(self.activity_version, "request activity version")?,
            submitted_at_unix: optional_i64_to_u64(
                self.submitted_at_unix,
                "request submission time",
            )?,
            closed_at_unix: optional_i64_to_u64(self.closed_at_unix, "request close time")?,
            closed_by_user_id: self.closed_by_user_id,
            merged_at_unix: optional_i64_to_u64(self.merged_at_unix, "request merge time")?,
            merged_by_user_id: self.merged_by_user_id,
            merged_head_oid: self.merged_head_oid,
            merged_main_oid: self.merged_main_oid,
            created_at_unix: i64_to_u64(self.created_at_unix, "request creation time")?,
            updated_at_unix: i64_to_u64(self.updated_at_unix, "request update time")?,
        };
        request.validate_facts()?;
        Ok(request)
    }
}
