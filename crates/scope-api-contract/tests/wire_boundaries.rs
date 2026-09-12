use scope_api_contract::*;

/// Each body fails to compile if a public payload field leaks a domain type
/// instead of the owned wire type named here.
#[test]
#[allow(dead_code)]
fn public_payload_fields_are_owned_wire_types() {
    fn session(response: CliSessionTokenResponse) {
        let _: SessionIdentity = response.identity;
    }
    fn repository(
        summary: RepoSummaryResponse,
        access: RepositoryAccessResponse,
        token: FirstPushTokenResponse,
        config: RepoConfigResponse,
    ) {
        let _: RepoLifecycleState = summary.lifecycle_state;
        let _: RepositoryActor = access.actor;
        let _: FirstPushTokenStatus = token.status;
        let _: RepoConfig = config.config;
    }
    fn request(
        summary: RequestSummaryResponse,
        list_item: RequestListItemResponse,
        event: RequestEventResponse,
        discussion: RequestDiscussionSummaryResponse,
        file: CommitFileResponse,
    ) {
        let _: RequestActorRole = summary.author_role;
        let _: RequestAudience = summary.audience;
        let _: RequestState = summary.state;
        let _: RequestMergeabilityStatus = summary.mergeability.status;
        let _: RequestState = list_item.state;
        let _: RequestEventKind = event.kind;
        let _: RequestEventPayload = event.payload;
        let _: RequestDiscussionStatus = discussion.status;
        let _: FileChangeKind = file.kind;
        let _: Visibility = file.visibility;
    }
    fn repository_events(event: RepoChangeEvent) {
        let _: RepoChangeKind = event.kind;
    }
    fn runtime(
        status: AttemptStatusResponse,
        step: AttemptStepStatusResponse,
        preparation: AttemptCachePreparationReport,
        finalization: AttemptCacheFinalizationReport,
    ) {
        let _: AttemptState = status.state;
        let _: StepState = step.state;
        let _: CachePreparation = preparation.preparation;
        let _: CacheFinalState = finalization.final_state;
    }
    fn repository_run(
        summary: RepositoryRunSummaryResponse,
        attempt: RepositoryRunAttemptResponse,
        step: RepositoryRunStepResponse,
        cache: RepositoryRunCacheObservationResponse,
    ) {
        let _: RepositoryRunTrigger = summary.trigger;
        let _: RunState = summary.state;
        let _: AttemptState = attempt.state;
        let _: Option<RepositoryRunTerminalReason> = attempt.terminal_reason;
        let _: StepState = step.state;
        let _: CachePreparation = cache.preparation;
        let _: CacheFinalState = cache.final_state;
    }
}
