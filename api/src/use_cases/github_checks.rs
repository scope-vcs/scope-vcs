use crate::{
    error::ApiError,
    git::command::{run_git_output, successful_git_output},
    state::AppState,
};
use scope_domain::{
    github_check_catalog::{
        GitHubCheckCatalog, GitHubCheckSources, GitHubWorkflowFile, WorkflowValue,
        github_check_catalog,
    },
    github_connection::GitHubConnection,
    requests::GitHubBranch,
};
use std::{collections::HashMap, path::Path};
use yaml_rust2::{
    Yaml, YamlLoader,
    parser::{Event, EventReceiver, Parser},
};

const WORKFLOWS_DIRECTORY: &str = ".github/workflows/";
const MAX_WORKFLOW_BYTES: u64 = 512 * 1024;
const MAX_WORKFLOW_NODES: usize = 100_000;
const MAX_WORKFLOW_EXPANDED_BYTES: usize = 4 * 1024 * 1024;

pub(crate) async fn read_github_check_catalog(
    state: &AppState,
    connection: &GitHubConnection,
) -> Result<GitHubCheckCatalog, ApiError> {
    let app = state.github_app()?;
    let installation_id = connection.installation_id;
    let full_name = &connection.github_full_name;
    let main = GitHubBranch::Main.name();
    let (workflow_files, branch, workflow_states) = tokio::try_join!(
        main_workflow_files(state, &connection.repository_id),
        app.branch_checks(installation_id, full_name, &main),
        app.workflow_states(installation_id, full_name),
    )?;
    let reported_on_main = match &branch {
        Some(branch) => app
            .commit_check_runs(installation_id, full_name, &branch.head_oid)
            .await?
            .unwrap_or_default()
            .into_iter()
            .map(|run| run.name)
            .collect(),
        None => Vec::new(),
    };
    Ok(github_check_catalog(GitHubCheckSources {
        workflow_files,
        workflow_states,
        required_on_github: branch.map(|branch| branch.required).unwrap_or_default(),
        reported_on_main,
        opens_pull_requests: connection.review_bots.open_pull_requests,
    }))
}

async fn main_workflow_files(
    state: &AppState,
    repo_id: &str,
) -> Result<Vec<GitHubWorkflowFile>, ApiError> {
    let Some(incarnation) = state.repository_incarnation(repo_id).await? else {
        return Ok(Vec::new());
    };
    let Some(head) = state
        .metadata
        .repositories()
        .repository_content_source(&incarnation)
        .await?
    else {
        return Ok(Vec::new());
    };
    let repo = state
        .repository_engine
        .materialize_repository(state, &incarnation, &head)
        .await?;
    crate::git::blocking::run(move || workflow_files_at(repo.as_ref(), &head.head_oid)).await
}

fn workflow_files_at(repo: &Path, commit: &str) -> Result<Vec<GitHubWorkflowFile>, ApiError> {
    const ACTION: &str = "listing GitHub workflow files";
    let listing = successful_git_output(
        run_git_output(
            Some(repo),
            &["ls-tree", "-z", "-l", commit, "--", WORKFLOWS_DIRECTORY],
            ACTION,
        )?,
        ACTION,
    )?;
    listing
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(workflow_entry)
        .map(|(path, oid, size)| {
            Ok(GitHubWorkflowFile {
                document: if size > MAX_WORKFLOW_BYTES {
                    Err("The file is larger than 512 KiB.".to_string())
                } else {
                    parse_workflow(&read_blob(repo, &oid)?)
                },
                path,
            })
        })
        .collect()
}

fn workflow_entry(entry: &[u8]) -> Option<(String, String, u64)> {
    let (meta, path) = std::str::from_utf8(entry).ok()?.split_once('\t')?;
    let [_mode, kind, oid, size] = meta.split_whitespace().collect::<Vec<_>>()[..] else {
        return None;
    };
    if kind != "blob" || !(path.ends_with(".yml") || path.ends_with(".yaml")) {
        return None;
    }
    Some((path.to_string(), oid.to_string(), size.parse().ok()?))
}

fn read_blob(repo: &Path, oid: &str) -> Result<Vec<u8>, ApiError> {
    const ACTION: &str = "reading a GitHub workflow file";
    Ok(successful_git_output(
        run_git_output(Some(repo), &["cat-file", "blob", oid], ACTION)?,
        ACTION,
    )?
    .stdout)
}

fn parse_workflow(bytes: &[u8]) -> Result<WorkflowValue, String> {
    let text = std::str::from_utf8(bytes).map_err(|_| "The file is not UTF-8 text.".to_string())?;
    let mut nodes = ExpandedNodes::default();
    Parser::new_from_str(text)
        .load(&mut nodes, true)
        .map_err(|error| error.to_string())?;
    if nodes.exceeded {
        return Err("The file expands to more values than Scope reads.".to_string());
    }
    let documents = YamlLoader::load_from_str(text).map_err(|error| error.to_string())?;
    documents
        .into_iter()
        .next()
        .map_or(Ok(WorkflowValue::Null), workflow_value)
}

#[derive(Clone, Copy, Default)]
struct ExpandedSize {
    nodes: usize,
    bytes: usize,
}

impl ExpandedSize {
    fn scalar(text: &str) -> Self {
        Self {
            nodes: 1,
            bytes: text.len(),
        }
    }

    fn plus(self, other: Self) -> Self {
        Self {
            nodes: self.nodes.saturating_add(other.nodes),
            bytes: self.bytes.saturating_add(other.bytes),
        }
    }

    fn within_budget(self) -> bool {
        self.nodes <= MAX_WORKFLOW_NODES && self.bytes <= MAX_WORKFLOW_EXPANDED_BYTES
    }
}

#[derive(Default)]
struct ExpandedNodes {
    anchors: HashMap<usize, ExpandedSize>,
    open: Vec<(usize, ExpandedSize)>,
    total: ExpandedSize,
    exceeded: bool,
}

impl ExpandedNodes {
    fn add(&mut self, size: ExpandedSize) {
        match self.open.last_mut() {
            Some((_, open)) => *open = open.plus(size),
            None => self.total = self.total.plus(size),
        }
        let expanded = self
            .open
            .iter()
            .fold(self.total, |expanded, (_, open)| expanded.plus(*open));
        if !expanded.within_budget() {
            self.exceeded = true;
        }
    }

    fn close(&mut self) {
        if let Some((anchor, size)) = self.open.pop() {
            if anchor > 0 {
                self.anchors.insert(anchor, size);
            }
            self.add(size);
        }
    }
}

impl EventReceiver for ExpandedNodes {
    fn on_event(&mut self, event: Event) {
        if self.exceeded {
            return;
        }
        match event {
            Event::Scalar(text, _, anchor, _) => {
                let size = ExpandedSize::scalar(&text);
                if anchor > 0 {
                    self.anchors.insert(anchor, size);
                }
                self.add(size);
            }
            Event::Alias(anchor) => {
                let size = self.anchors.get(&anchor).copied().unwrap_or_default();
                self.add(size.plus(ExpandedSize { nodes: 1, bytes: 0 }));
            }
            Event::SequenceStart(anchor, _) | Event::MappingStart(anchor, _) => {
                self.open
                    .push((anchor, ExpandedSize { nodes: 1, bytes: 0 }));
            }
            Event::SequenceEnd | Event::MappingEnd => self.close(),
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
    }
}

fn workflow_value(yaml: Yaml) -> Result<WorkflowValue, String> {
    Ok(match yaml {
        Yaml::Null => WorkflowValue::Null,
        Yaml::Boolean(value) => WorkflowValue::Bool(value),
        Yaml::Integer(value) => WorkflowValue::Integer(value),
        Yaml::Real(text) => text
            .parse()
            .map_or(WorkflowValue::String(text), WorkflowValue::Real),
        Yaml::String(value) => WorkflowValue::String(value),
        Yaml::Array(items) => WorkflowValue::Sequence(
            items
                .into_iter()
                .map(workflow_value)
                .collect::<Result<_, _>>()?,
        ),
        Yaml::Hash(entries) => WorkflowValue::Mapping(
            entries
                .into_iter()
                .map(|(key, value)| Ok((mapping_key(key)?, workflow_value(value)?)))
                .collect::<Result<_, String>>()?,
        ),
        Yaml::Alias(_) | Yaml::BadValue => {
            return Err("The file has a value GitHub cannot read.".to_string());
        }
    })
}

fn mapping_key(key: Yaml) -> Result<String, String> {
    match key {
        Yaml::String(key) | Yaml::Real(key) => Ok(key),
        Yaml::Integer(key) => Ok(key.to_string()),
        Yaml::Boolean(key) => Ok(key.to_string()),
        Yaml::Null => Ok("null".to_string()),
        Yaml::Array(_) | Yaml::Hash(_) | Yaml::Alias(_) | Yaml::BadValue => {
            Err("The file uses a mapping key GitHub cannot read.".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workflow_yaml_keeps_key_order_resolves_anchors_and_reports_unreadable_files() {
        let parsed = parse_workflow(
            b"on:\n  push:\n    branches: &scope ['scope/**']\nversions: [3.10, 18, true]\nbranches: *scope\n",
        )
        .unwrap();
        let scope = WorkflowValue::Sequence(vec![WorkflowValue::String("scope/**".into())]);
        assert_eq!(
            parsed,
            WorkflowValue::Mapping(vec![
                (
                    "on".into(),
                    WorkflowValue::Mapping(vec![(
                        "push".into(),
                        WorkflowValue::Mapping(vec![("branches".into(), scope.clone())]),
                    )]),
                ),
                (
                    "versions".into(),
                    WorkflowValue::Sequence(vec![
                        WorkflowValue::Real(3.1),
                        WorkflowValue::Integer(18),
                        WorkflowValue::Bool(true),
                    ]),
                ),
                ("branches".into(), scope),
            ])
        );
        assert!(parse_workflow(b"on: [push\n").is_err());
        assert!(parse_workflow(&[0xff, 0xfe]).is_err());
        assert!(parse_workflow(b"? [a, b]\n: c\n").is_err());
        let mut nested = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
        for level in 1..8 {
            let previous = level - 1;
            nested.push_str(&format!(
                "b{level}: &b{level} [*{}, *{0}, *{0}, *{0}, *{0}, *{0}, *{0}, *{0}, *{0}, *{0}]\n",
                if previous == 0 {
                    "a".to_string()
                } else {
                    format!("b{previous}")
                }
            ));
        }
        let long = format!(
            "s: &s {}\nr: &r [{}]\nt: [{}]\n",
            "x".repeat(256 * 1024),
            vec!["*s"; 100].join(", "),
            vec!["*r"; 900].join(", ")
        );
        for expanding in [nested, long] {
            assert_eq!(
                parse_workflow(expanding.as_bytes()),
                Err("The file expands to more values than Scope reads.".to_string())
            );
        }
    }
}
