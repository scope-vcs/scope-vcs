use super::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_request_revision_context_receipts")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub request_id: String,
    #[sea_orm(primary_key, auto_increment = false)]
    pub operation_key: String,
    pub payload_digest: String,
    pub revision_id: String,
    pub created_at_unix: i64,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
