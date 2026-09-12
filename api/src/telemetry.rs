use std::sync::OnceLock;

static REPLICA_ID: OnceLock<String> = OnceLock::new();

pub(crate) fn replica_id() -> &'static str {
    REPLICA_ID
        .get_or_init(|| {
            std::env::var("RAILWAY_REPLICA_ID")
                .or_else(|_| std::env::var("HOSTNAME"))
                .unwrap_or_else(|_| "local".to_string())
        })
        .as_str()
}

pub(crate) fn request_trace_id() -> String {
    let mut random = [0_u8; 8];
    match getrandom::fill(&mut random) {
        Ok(()) => hex::encode(random),
        Err(error) => {
            static LOGGED: OnceLock<()> = OnceLock::new();
            LOGGED.get_or_init(|| tracing::warn!(%error, "failed to generate request trace id"));
            String::new()
        }
    }
}
