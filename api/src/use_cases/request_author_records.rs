use crate::{error::ApiError, state::AppState};
use scope_domain::{
    repository::access::RepositoryAccessContext,
    requests::{AuthorRecord, AuthoredRequest, author_record, author_record_subject},
};
use scope_postgres::db::RequestListRow;
use std::collections::HashMap;

pub(crate) async fn author_records(
    state: &AppState,
    repo: &RepositoryAccessContext,
    requests: &[RequestListRow],
) -> Result<HashMap<String, AuthorRecord>, ApiError> {
    let subjects = requests
        .iter()
        .filter_map(|request| {
            let authored = AuthoredRequest {
                author_user_id: request.author_user_id.as_deref(),
                author_role: request.author_role,
                state: request.state,
            };
            let author = author_record_subject(authored, &repo.access, &repo.views)?;
            Some((request, authored, author))
        })
        .collect::<Vec<_>>();
    let mut authors = subjects
        .iter()
        .map(|(_, _, author)| author.to_string())
        .collect::<Vec<_>>();
    authors.sort_unstable();
    authors.dedup();
    let totals = state
        .metadata
        .requests()
        .author_record_totals(&repo.record.id, authors)
        .await?;
    let mut records = HashMap::new();
    for (request, authored, author) in subjects {
        let author_totals = totals.get(author).copied().unwrap_or_default();
        if let Some(record) = author_record(authored, &repo.access, &repo.views, author_totals)? {
            records.insert(request.id.clone(), record);
        }
    }
    Ok(records)
}
