use scope_api_contract::*;

#[test]
fn public_payload_fields_are_owned_wire_types() {
    let _session = |response: CliSessionTokenResponse| {
        let _: SessionIdentity = response.identity;
    };
    let _repository = |summary: RepoSummaryResponse,
                       access: RepositoryAccessResponse,
                       token: FirstPushTokenResponse,
                       config: RepoConfigResponse| {
        let _: RepoLifecycleState = summary.lifecycle_state;
        let _: RepositoryActor = access.actor;
        let _: FirstPushTokenStatus = token.status;
        let _: RepoConfig = config.config;
    };
    let _request = |summary: RequestSummaryResponse,
                    list_item: RequestListItemResponse,
                    event: RequestEventResponse,
                    discussion: RequestDiscussionSummaryResponse,
                    file: CommitFileResponse| {
        let _: RequestActorRole = summary.author_role;
        let _: ViewId = summary.view;
        let _: RequestState = summary.state;
        let _: RequestMergeabilityStatus = summary.mergeability.status;
        let _: RequestState = list_item.state;
        let _: RequestEventKind = event.kind;
        let _: RequestEventPayload = event.payload;
        let _: RequestDiscussionStatus = discussion.status;
        let _: FileChangeKind = file.kind;
        let _: ViewId = file.label;
    };
    let _request_auto_merge = |response: RequestAutoMergeResponse| {
        if let Some(intent) = response.intent {
            let _: RequestActorSummaryResponse = intent.actor;
            let _: RequestAutoMergeIntentStatus = intent.status;
            let _: Option<RequestAutoMergeStopReason> = intent.reason;
        }
    };
    let _repository_events = |event: RepoChangeEvent| {
        let _: RepoChangeKind = event.kind;
    };
    let _runtime = |status: AttemptStatusResponse,
                    step: AttemptStepStatusResponse,
                    preparation: AttemptCachePreparationReport,
                    finalization: AttemptCacheFinalizationReport| {
        let _: AttemptState = status.state;
        let _: StepState = step.state;
        let _: CachePreparation = preparation.preparation;
        let _: CacheFinalState = finalization.final_state;
    };
    let _repository_run = |summary: RepositoryRunSummaryResponse,
                           attempt: RepositoryRunAttemptResponse,
                           step: RepositoryRunStepResponse,
                           cache: RepositoryRunCacheObservationResponse| {
        let _: RepositoryRunTrigger = summary.trigger;
        let _: RunState = summary.state;
        let _: AttemptState = attempt.state;
        let _: Option<RepositoryRunTerminalReason> = attempt.terminal_reason;
        let _: StepState = step.state;
        let _: CachePreparation = cache.preparation;
        let _: CacheFinalState = cache.final_state;
    };
}
