use crate::{auth::scope::require_scope_user, error::ApiError, state::AppState};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use scope_api_contract::{
    RepositoryDependencyCheckResponse, RepositoryDependencyCheckStatus,
    RepositoryDependencyFindingResponse, RepositoryDependencyGapResponse,
    RepositoryDependencyReportResponse,
};
use scope_domain::dependency_analysis::{DependencyCheck, DependencyCheckStatus};

pub(crate) async fn get_repository_dependencies(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<Json<RepositoryDependencyCheckResponse>, ApiError> {
    let user = require_scope_user(&state, &headers).await?;
    let check = state
        .metadata
        .repositories()
        .dependency_check(&owner, &repo_name, &user.id)
        .await?
        .ok_or_else(|| ApiError::not_found("repository dependency check not found"))?;
    Ok(Json(dependency_check_response(check)))
}

fn dependency_check_response(check: DependencyCheck) -> RepositoryDependencyCheckResponse {
    RepositoryDependencyCheckResponse {
        status: match check.status {
            DependencyCheckStatus::Pending => RepositoryDependencyCheckStatus::Pending,
            DependencyCheckStatus::Ready => RepositoryDependencyCheckStatus::Ready,
            DependencyCheckStatus::Updating => RepositoryDependencyCheckStatus::Updating,
            DependencyCheckStatus::Failed => RepositoryDependencyCheckStatus::Failed,
            DependencyCheckStatus::Unsupported => RepositoryDependencyCheckStatus::Unsupported,
        },
        report: check
            .report
            .map(|report| RepositoryDependencyReportResponse {
                commit_oid: report.commit_oid,
                analyzer_version: report.analyzer_version,
                analyzed_file_count: report.analyzed_file_count,
                unsupported_files: report.unsupported_files,
                gaps: report
                    .gaps
                    .into_iter()
                    .map(|gap| RepositoryDependencyGapResponse {
                        path: gap.path,
                        reason: gap.reason,
                    })
                    .collect(),
                findings: report
                    .findings
                    .into_iter()
                    .map(|finding| RepositoryDependencyFindingResponse {
                        source_path: finding.source_path,
                        target_path: finding.target_path,
                    })
                    .collect(),
                public_file_count: report.public_file_count,
            }),
        error: check.error,
    }
}
