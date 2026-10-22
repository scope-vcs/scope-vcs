use super::*;
use crate::db::{
    ApplyContentOnlyPushCommand, LandedRequestCandidate, LandedRequestCompletion,
    RepositoryMutation, RepositoryMutationResult,
};

#[derive(Clone, Copy)]
pub(super) enum PushPath {
    ContentOnly,
    General,
}

pub(super) const PUSH_PATHS: [PushPath; 2] = [PushPath::ContentOnly, PushPath::General];

pub(super) async fn push(
    store: &crate::db::MetadataStore,
    path: PushPath,
    prepared: MergePreparation,
    candidates: Vec<LandedRequestCandidate>,
) -> Result<RepositoryMutationResult<GitHead>, String> {
    let input = scope_domain::runs::trigger::PushTriggerInput::from(&prepared.workflow_catalog);
    match path {
        PushPath::ContentOnly => {
            let repo = store
                .repositories()
                .repository_for_tests("owner/repo")
                .await
                .unwrap()
                .unwrap();
            store
                .repositories()
                .apply_content_only_push(ApplyContentOnlyPushCommand {
                    incarnation: repo.incarnation(),
                    owner: "owner".into(),
                    name: "repo".into(),
                    author_id: "user_owner".into(),
                    expected_git_frontier: prepared.expected_git_frontier,
                    update: prepared.update,
                    landing_file_mutation: RepositoryLandingFileMutation::Unchanged,
                    workflow_catalog: prepared.workflow_catalog,
                    push_trigger_input: input,
                    landed_request_candidates: candidates,
                    now_unix: 8,
                })
                .await
                .map(|result| result.unwrap())
                .map_err(|error| error.to_string())
        }
        PushPath::General => store
            .repositories()
            .mutate_repository("owner", "repo", 8, |repo| {
                let head = prepared.update.git_head.clone();
                apply_reviewed_update_to_repo(repo, prepared.update)
                    .map_err(scope_domain::repo_actions::reviewed_update_domain_error)?;
                let mut mutation = RepositoryMutation::with_push_trigger_input(
                    head,
                    input,
                    RepositoryLandingFileMutation::Unchanged,
                    prepared.workflow_catalog,
                );
                mutation.landed_requests = Some(LandedRequestCompletion {
                    actor_user_id: "user_owner".into(),
                    candidates,
                });
                Ok(mutation)
            })
            .await
            .map_err(|error| error.to_string()),
    }
}
