use super::{
    GITHUB_MAIN_PULL_REQUESTS_MESSAGE, GITHUB_PATH_FILTER_MESSAGE,
    GITHUB_STACKED_PULL_REQUESTS_MESSAGE, GitHubCheckRuns, WorkflowValue, branch_filter,
};

const PULL_REQUEST_UPDATE_TYPES: [&str; 3] = ["opened", "synchronize", "reopened"];

pub(super) struct ScopePushes {
    pub(super) request_branch: String,
    pub(super) base_branch: String,
    pub(super) opens_pull_requests: bool,
}

pub(super) type Event<'a> = (String, Option<&'a WorkflowValue>);

pub(super) fn events(document: &WorkflowValue) -> Vec<Event<'_>> {
    match document.get("on") {
        Some(WorkflowValue::String(event)) => vec![(event.clone(), None)],
        Some(WorkflowValue::Sequence(items)) => items
            .iter()
            .filter_map(WorkflowValue::scalar_text)
            .map(|event| (event, None))
            .collect(),
        Some(WorkflowValue::Mapping(entries)) => entries
            .iter()
            .map(|(event, config)| (event.clone(), Some(config)))
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn only_reusable(events: &[Event<'_>]) -> bool {
    !events.is_empty() && events.iter().all(|(event, _)| event == "workflow_call")
}

impl ScopePushes {
    pub(super) fn verdict(&self, events: &[Event<'_>]) -> GitHubCheckRuns {
        let triggered = events
            .iter()
            .filter_map(|(event, config)| match event.as_str() {
                "push" => push(*config, &self.request_branch),
                "pull_request" if self.opens_pull_requests => {
                    pull_request(*config, &self.base_branch, &self.request_branch)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        if triggered.contains(&GitHubCheckRuns::Yes) {
            return GitHubCheckRuns::Yes;
        }
        if let Some(maybe) = triggered
            .into_iter()
            .find(|runs| matches!(runs, GitHubCheckRuns::Maybe(_)))
        {
            return maybe;
        }
        let suggested = self
            .request_branch
            .split('/')
            .next()
            .map(|root| format!("{root}/**"))
            .unwrap_or_default();
        GitHubCheckRuns::No(format!(
            "This workflow has no trigger for Scope's request branches. \
             Add a push trigger for `{suggested}`."
        ))
    }
}

fn push(config: Option<&WorkflowValue>, branch: &str) -> Option<GitHubCheckRuns> {
    let Some(config @ WorkflowValue::Mapping(_)) = config else {
        return Some(GitHubCheckRuns::Yes);
    };
    let triggered = match branch_triggers(config, branch) {
        Some(triggered) => triggered,
        None => config.get("tags").is_none() && config.get("tags-ignore").is_none(),
    };
    triggered.then(|| path_verdict(config))
}

fn pull_request(
    config: Option<&WorkflowValue>,
    main_base: &str,
    stacked_base: &str,
) -> Option<GitHubCheckRuns> {
    let Some(config @ WorkflowValue::Mapping(_)) = config else {
        return Some(GitHubCheckRuns::Yes);
    };
    let type_triggered = config.get("types").is_none_or(|types| {
        branch_filter::filter_values(types)
            .iter()
            .any(|kind| PULL_REQUEST_UPDATE_TYPES.contains(&kind.as_str()))
    });
    if !type_triggered {
        return None;
    }
    let into = |base| branch_triggers(config, base).unwrap_or(true);
    match (into(main_base), into(stacked_base)) {
        (true, true) => Some(path_verdict(config)),
        (true, false) => Some(GitHubCheckRuns::Maybe(
            GITHUB_MAIN_PULL_REQUESTS_MESSAGE.to_string(),
        )),
        (false, true) => Some(GitHubCheckRuns::Maybe(
            GITHUB_STACKED_PULL_REQUESTS_MESSAGE.to_string(),
        )),
        (false, false) => None,
    }
}

fn branch_triggers(config: &WorkflowValue, branch: &str) -> Option<bool> {
    if let Some(branches) = config.get("branches") {
        return Some(branch_filter::includes(branches, branch));
    }
    config
        .get("branches-ignore")
        .map(|ignored| !branch_filter::includes(ignored, branch))
}

fn path_verdict(config: &WorkflowValue) -> GitHubCheckRuns {
    if config.get("paths").is_some() || config.get("paths-ignore").is_some() {
        GitHubCheckRuns::Maybe(GITHUB_PATH_FILTER_MESSAGE.to_string())
    } else {
        GitHubCheckRuns::Yes
    }
}
