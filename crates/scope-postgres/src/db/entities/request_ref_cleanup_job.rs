use sea_orm::entity::prelude::*;

#[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
#[sea_orm(table_name = "scope_request_ref_cleanup_jobs")]
pub struct Model {
    #[sea_orm(primary_key, auto_increment = false)]
    pub id: String,
    pub repo_id: String,
    pub incarnation_id: String,
    pub request_id: String,
    pub request_name: String,
    pub head_oid: String,
    pub created_at_unix: i64,
    pub next_run_at_unix: i64,
    pub attempts: i32,
    pub last_error: Option<String>,
}

#[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
pub enum Relation {}

impl ActiveModelBehavior for ActiveModel {}
