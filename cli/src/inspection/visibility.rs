//! Local visibility diagnostics for status and doctor.
use super::{DiagnosticState, Report, local::local_visibility, record};
use crate::{git_repo::GitRepo, repo_config};
use repo_config::WorktreeRepoConfigPresence;

pub(super) fn inspect_visibility(report: &mut Report, repo: &GitRepo) {
    let presence = repo_config::worktree_scope_repo_config_presence(&repo.root);
    match presence {
        Ok(WorktreeRepoConfigPresence::Absent) => match repo_config::is_linked_worktree(&repo.root)
        {
            Ok(true) => record(
                report,
                "visibility",
                DiagnosticState::Info,
                "This linked worktree has no local visibility config; request commands do not require it".into(),
                Some("Run scope pull to load this repository's visibility config without changing an origin-tracking branch".into()),
            ),
            Ok(false) => record(
                report,
                "visibility",
                DiagnosticState::Problem,
                "Local visibility config is missing".into(),
                Some("Run scope pull for an existing Scope repository, or scope init for a new one".into()),
            ),
            Err(error) => record(
                report,
                "visibility",
                DiagnosticState::Problem,
                error.to_string(),
                Some("Repair the local Git worktree state before publishing".into()),
            ),
        },
        Ok(_) => match local_visibility(repo) {
            Ok(visibility) => {
                let message = if visibility.local_edits == Some(true) {
                    "Local visibility changes have not been published"
                } else {
                    "Local visibility config is valid"
                };
                record(report, "visibility", DiagnosticState::Ok, message.into(), None);
                if let Some(local) = &mut report.local {
                    local.visibility = Some(visibility);
                }
            }
            Err(error) => record(
                report,
                "visibility",
                DiagnosticState::Problem,
                error.to_string(),
                Some("Inspect scope visibility show and repair the partial or invalid local visibility state before publishing".into()),
            ),
        },
        Err(error) => record(
            report,
            "visibility",
            DiagnosticState::Problem,
            error.to_string(),
            Some("Repair the local visibility state path before publishing".into()),
        ),
    }
}
