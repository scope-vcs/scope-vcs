pub use scope_domain::git_oid::{GitOid, GitOidParseError};

#[cfg(feature = "ts")]
#[derive(ts_rs::TS)]
#[ts(rename = "GitOid", type = "string")]
pub struct GitOidSchema;

#[cfg(feature = "ts")]
impl schemars::JsonSchema for GitOidSchema {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GitOid".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{40}$"
        })
    }
}
