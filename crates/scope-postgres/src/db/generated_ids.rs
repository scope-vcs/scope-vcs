use crate::error::PostgresError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GeneratedIdKind {
    CleanupGeneration,
    DependencyAnalysisLease,
    OutboxJob,
    RepositoryIncarnation,
    RequestAutoMergeClaim,
    RequestMergedEvent,
}

impl GeneratedIdKind {
    fn prefix(self) -> &'static str {
        match self {
            Self::CleanupGeneration => "",
            Self::DependencyAnalysisLease => "dependency_",
            Self::OutboxJob => "outbox_",
            Self::RepositoryIncarnation => "repoi_",
            Self::RequestAutoMergeClaim => "auto_merge_claim_",
            Self::RequestMergedEvent => "event_request_merged_",
        }
    }
}

pub(crate) fn generate_id(kind: GeneratedIdKind) -> Result<String, PostgresError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| {
        PostgresError::internal_message(format!("failed to generate {kind:?}: {error}"))
    })?;
    Ok(format!("{}{}", kind.prefix(), hex::encode(random)))
}
