//! The destructive local seed owns a dedicated loopback database and its public schema.

use sqlx::postgres::PgConnectOptions;
use std::{fmt, net::IpAddr};

#[derive(Clone)]
pub struct LocalDevDatabase {
    pub(crate) options: PgConnectOptions,
    pub(crate) url: String,
}

impl fmt::Debug for LocalDevDatabase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalDevDatabase")
            .field("host", &self.options.get_host())
            .field("database", &self.options.get_database())
            .finish_non_exhaustive()
    }
}

impl LocalDevDatabase {
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        let mut url = url::Url::parse(raw)
            .map_err(|_| anyhow::anyhow!("DATABASE_URL must be a PostgreSQL URL"))?;
        if !matches!(url.scheme(), "postgres" | "postgresql") || url.fragment().is_some() {
            anyhow::bail!("DATABASE_URL must be a PostgreSQL URL without a fragment");
        }
        if url.query_pairs().any(|(key, _)| {
            matches!(
                key.as_ref(),
                "search_path" | "schema" | "current_schema" | "currentSchema"
            )
        }) {
            anyhow::bail!(
                "DATABASE_URL schema aliases are unsupported; use a dedicated Scope local/dev database"
            );
        }
        // SQLx logs the key and value of ignored parameters. Reject them before
        // parsing so a misspelled credential option cannot leak through tracing.
        if url.query_pairs().any(|(key, _)| {
            !matches!(
                key.as_ref(),
                "host"
                    | "hostaddr"
                    | "port"
                    | "dbname"
                    | "user"
                    | "password"
                    | "sslmode"
                    | "ssl-mode"
                    | "sslrootcert"
                    | "ssl-root-cert"
                    | "ssl-ca"
                    | "sslcert"
                    | "ssl-cert"
                    | "sslkey"
                    | "ssl-key"
                    | "application_name"
                    | "statement-cache-capacity"
            )
        }) {
            anyhow::bail!("DATABASE_URL contains an unsupported local connection parameter");
        }

        // SQLx applies query overrides and environment defaults. Validate those exact
        // options, then use them directly for the seed connection.
        let options: PgConnectOptions = url.as_str().parse()?;
        let host = options.get_host().trim_matches(['[', ']']);
        let loopback = host.eq_ignore_ascii_case("localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback());
        if !loopback || options.get_socket().is_some() {
            anyhow::bail!("DATABASE_URL must target localhost over TCP in local dev");
        }
        let database = options.get_database().unwrap_or_default();
        if !is_local_database(database) {
            anyhow::bail!("DATABASE_URL must target a dedicated Scope local/dev database");
        }
        if options.get_options().is_some_and(|value| !value.is_empty()) {
            anyhow::bail!(
                "DATABASE_URL and PGOPTIONS must not override local seed options; local seeding uses the public schema"
            );
        }
        // Pin the schema even when the database role has a custom search_path.
        // The URL is retained for maintenance and dedicated listener connections.
        url.query_pairs_mut()
            .append_pair("options[search_path]", "public");
        Ok(Self {
            options: options.options([("search_path", "public")]),
            url: url.into(),
        })
    }

    pub fn into_url(self) -> String {
        self.url
    }
}

fn is_local_database(name: &str) -> bool {
    [
        "scope_dev",
        "scope-dev",
        "scope_local",
        "scope-local",
        "scope_vcs_dev",
        "scope-vcs-dev",
        "scope_vcs_local",
        "scope-vcs-local",
        "scope_test",
        "scope-test",
    ]
    .iter()
    .any(|prefix| {
        name == *prefix
            || name
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('_') || suffix.starts_with('-'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_target_overrides_cannot_escape_the_local_database() {
        for raw in [
            "postgres://localhost/scope_dev?host=db.example.net",
            "postgres://localhost/scope_dev?hostaddr=192.0.2.10",
            "postgres://localhost/scope_dev?dbname=customer_data",
            "postgres://localhost/scope_dev?host=%2Ftmp",
            "postgres://localhost/customer_scope_dev",
            "postgres://127.0.0.1.example.com/scope_dev",
        ] {
            assert!(LocalDevDatabase::parse(raw).is_err(), "accepted {raw}");
        }
    }

    #[test]
    fn aliases_and_session_options_cannot_authorize_seeding() {
        for query in [
            "search_path=scope_test",
            "schema=scope_test",
            "current_schema=scope_test",
            "currentSchema=scope_test",
            "options[search_path]=scope_test",
            "options=-c%20search_path%3Dscope_test",
        ] {
            assert!(
                LocalDevDatabase::parse(&format!("postgres://localhost/postgres?{query}")).is_err()
            );
            assert!(
                LocalDevDatabase::parse(&format!("postgres://localhost/scope_dev?{query}"))
                    .is_err()
            );
        }
    }

    #[test]
    fn unknown_connection_parameters_are_rejected_without_exposing_values() {
        let error = LocalDevDatabase::parse("postgres://localhost/scope_dev?token=private-value")
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "DATABASE_URL contains an unsupported local connection parameter"
        );
    }

    #[test]
    fn loopback_targets_pin_the_same_effective_schema_for_all_connections() {
        for raw in [
            "postgres://scope@localhost/scope_dev",
            "postgres://scope@127.0.0.1/scope_test_suite",
            "postgres://scope@[::1]/scope_local",
            "postgres://scope@elsewhere/other?host=127.0.0.1&dbname=scope_dev",
        ] {
            let target = LocalDevDatabase::parse(raw).unwrap();
            let reconnect: PgConnectOptions = target.url.parse().unwrap();
            assert_eq!(target.options.get_host(), reconnect.get_host());
            assert_eq!(target.options.get_database(), reconnect.get_database());
            assert_eq!(target.options.get_options(), Some("-c search_path=public"));
            assert_eq!(target.options.get_options(), reconnect.get_options());
        }
    }
}
