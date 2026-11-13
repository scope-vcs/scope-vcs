use crate::{
    auth::viewer::Viewer,
    error::ApiError,
    http::{
        repos::{get_file_content, get_owner_repositories},
        request_checks::get_request_checks,
        request_discussions::{list_discussions, list_replies},
        request_review::{comparison::get_request_revision_file_diff, list_request_revisions},
        requests::{get_request, list_requests},
    },
    state::AppState,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use scope_domain::account::agent_access::AgentScope;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};

#[derive(Clone, Copy)]
pub(super) enum Operation {
    RepoList,
    RequestList,
    RequestShow,
    RequestDiff,
    RequestCi,
    DiscussionList,
    DiscussionReplies,
    FileRead,
}

pub(super) enum ToolError {
    Arguments(String),
    Api(ApiError),
}

impl From<ApiError> for ToolError {
    fn from(error: ApiError) -> Self {
        Self::Api(error)
    }
}

impl Operation {
    const ALL: [Self; 8] = [
        Self::RepoList,
        Self::RequestList,
        Self::RequestShow,
        Self::RequestDiff,
        Self::RequestCi,
        Self::DiscussionList,
        Self::DiscussionReplies,
        Self::FileRead,
    ];

    pub(super) fn named(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|operation| operation.name() == name)
    }

    fn name(self) -> &'static str {
        match self {
            Self::RepoList => "repo_list",
            Self::RequestList => "request_list",
            Self::RequestShow => "request_show",
            Self::RequestDiff => "request_diff",
            Self::RequestCi => "request_ci",
            Self::DiscussionList => "discussion_list",
            Self::DiscussionReplies => "discussion_replies",
            Self::FileRead => "file_read",
        }
    }

    pub(super) fn scope(self) -> AgentScope {
        match self {
            Self::RepoList
            | Self::RequestList
            | Self::RequestShow
            | Self::RequestDiff
            | Self::RequestCi
            | Self::DiscussionList
            | Self::DiscussionReplies
            | Self::FileRead => AgentScope::Read,
        }
    }

    fn description(self) -> &'static str {
        match self {
            Self::RepoList => {
                "List the repositories an owner has that you can read. Without an owner, lists your own."
            }
            Self::RequestList => {
                "List contribution requests in a repository you can read, newest first. Pass next_cursor back as cursor for the next page."
            }
            Self::RequestShow => {
                "Show one request: title, description, state, author, base, head commit, and line counts."
            }
            Self::RequestDiff => {
                "Show what a request changes. Without a path, lists the changed files in its latest revision and returns review_revision_id. With a path and that revision ID, shows the file's diff."
            }
            Self::RequestCi => {
                "Show CI for a request's current revision, its review bots, and whether it can merge."
            }
            Self::DiscussionList => {
                "List a request's discussions with each one's latest replies. Pass next_cursor back as cursor for the next page."
            }
            Self::DiscussionReplies => {
                "Read every reply in one discussion, including those discussion_list leaves out. Pass next_before_position back as before for older replies."
            }
            Self::FileRead => {
                "Read one file on a repository's main branch, as the view you can read shows it. Pass a request ID to read the request's version instead."
            }
        }
    }

    fn input_schema(self) -> Value {
        let repo = json!({ "type": "string", "description": "Repository as owner/name" });
        let request = json!({ "type": "string", "description": "Request ID, starting with req_" });
        let page = json!({
            "cursor": { "type": "string", "description": "next_cursor from the previous page" },
            "limit": { "type": "integer", "minimum": 1 },
        });
        let (properties, required) = match self {
            Self::RepoList => (
                json!({ "owner": { "type": "string", "description": "Owner handle; defaults to you" } }),
                json!([]),
            ),
            Self::RequestList => {
                let mut properties = page;
                properties["repo"] = repo;
                properties["name"] =
                    json!({ "type": "string", "description": "Only the request with this name" });
                (properties, json!(["repo"]))
            }
            Self::RequestShow | Self::RequestCi => (
                json!({ "repo": repo, "request": request }),
                json!(["repo", "request"]),
            ),
            Self::RequestDiff => (
                json!({
                    "repo": repo,
                    "request": request,
                    "path": { "type": "string", "description": "File path to diff, such as src/main.rs" },
                    "revision": { "type": "string", "description": "Revision ID; required with path, and selects the listing otherwise" },
                }),
                json!(["repo", "request"]),
            ),
            Self::DiscussionList => {
                let mut properties = page;
                properties["repo"] = repo;
                properties["request"] = request;
                (properties, json!(["repo", "request"]))
            }
            Self::DiscussionReplies => (
                json!({
                    "repo": repo,
                    "request": request,
                    "discussion": { "type": "string", "description": "Discussion ID from discussion_list" },
                    "before": { "type": "integer", "minimum": 0, "description": "next_before_position from the previous page" },
                    "limit": { "type": "integer", "minimum": 1 },
                }),
                json!(["repo", "request", "discussion"]),
            ),
            Self::FileRead => (
                json!({
                    "repo": repo,
                    "path": { "type": "string", "description": "File path, such as README.md" },
                    "view": { "type": "string", "description": "View to read through; defaults to yours" },
                    "request": { "type": "string", "description": "Read the file as this request changes it" },
                }),
                json!(["repo", "path"]),
            ),
        };
        json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
    }

    pub(super) async fn run(
        self,
        state: &AppState,
        viewer: Viewer,
        args: Value,
    ) -> Result<Value, ToolError> {
        let state = State(state.clone());
        match self {
            Self::RepoList => {
                let owner = match optional(&args, "owner")? {
                    Some(owner) => owner,
                    None => viewer
                        .account()
                        .map(|user| user.handle.clone())
                        .ok_or_else(|| ToolError::Arguments("owner is required".into()))?,
                };
                to_value(get_owner_repositories(state, Path(owner), viewer).await)
            }
            Self::RequestList => {
                let (owner, repo) = repository(&args)?;
                to_value(list_requests(state, Path((owner, repo)), query(&args)?, viewer).await)
            }
            Self::RequestShow => {
                to_value(get_request(state, Path(request_path(&args)?), viewer).await)
            }
            Self::RequestCi => {
                to_value(get_request_checks(state, Path(request_path(&args)?), viewer).await)
            }
            Self::DiscussionList => {
                let path = Path(request_path(&args)?);
                to_value(list_discussions(state, path, query(&args)?, viewer).await)
            }
            Self::DiscussionReplies => {
                let (owner, repo, request) = request_path(&args)?;
                let path = Path((owner, repo, request, required(&args, "discussion")?));
                to_value(list_replies(state, path, query(&args)?, viewer).await)
            }
            Self::RequestDiff => {
                let (owner, repo, request) = request_path(&args)?;
                if optional(&args, "path")?.is_none() {
                    let path = Path((owner, repo, request));
                    return to_value(
                        list_request_revisions(state, path, query(&args)?, viewer).await,
                    );
                }
                let path = Path((owner, repo, request, required(&args, "revision")?));
                to_value(get_request_revision_file_diff(state, path, query(&args)?, viewer).await)
            }
            Self::FileRead => {
                let (owner, repo) = repository(&args)?;
                to_value(get_file_content(state, Path((owner, repo)), query(&args)?, viewer).await)
            }
        }
    }
}

pub(super) fn definitions() -> Vec<Value> {
    Operation::ALL
        .into_iter()
        .map(|operation| {
            json!({
                "name": operation.name(),
                "description": operation.description(),
                "inputSchema": operation.input_schema(),
                "annotations": { "readOnlyHint": operation.scope() == AgentScope::Read },
            })
        })
        .collect()
}

pub(super) fn call_result(outcome: Result<Value, ToolError>) -> Value {
    match outcome {
        Ok(value) => json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "structuredContent": value,
            "isError": false,
        }),
        Err(ToolError::Arguments(message)) => json!({
            "content": [{ "type": "text", "text": message }],
            "isError": true,
        }),
        Err(ToolError::Api(error)) => {
            let (_, body) = error.into_public_parts();
            let text = match &body.instruction {
                Some(instruction) => format!("{} {instruction}", body.message),
                None => body.message.clone(),
            };
            json!({
                "content": [{ "type": "text", "text": text }],
                "structuredContent": body,
                "isError": true,
            })
        }
    }
}

fn optional(args: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        Some(_) => Err(ToolError::Arguments(format!("{key} must be a string"))),
    }
}

fn required(args: &Value, key: &str) -> Result<String, ToolError> {
    optional(args, key)?.ok_or_else(|| ToolError::Arguments(format!("{key} is required")))
}

fn repository(args: &Value) -> Result<(String, String), ToolError> {
    let repo = required(args, "repo")?;
    repo.split_once('/')
        .filter(|(owner, name)| !owner.is_empty() && !name.is_empty() && !name.contains('/'))
        .map(|(owner, name)| (owner.to_string(), name.to_string()))
        .ok_or_else(|| ToolError::Arguments(format!("repo must be owner/name, not {repo}")))
}

fn request_path(args: &Value) -> Result<(String, String, String), ToolError> {
    let (owner, repo) = repository(args)?;
    Ok((owner, repo, required(args, "request")?))
}

fn query<T: DeserializeOwned>(args: &Value) -> Result<Query<T>, ToolError> {
    serde_json::from_value(args.clone())
        .map(Query)
        .map_err(|error| ToolError::Arguments(format!("invalid arguments: {error}")))
}

fn to_value<T: Serialize>(response: Result<Json<T>, ApiError>) -> Result<Value, ToolError> {
    let Json(body) = response?;
    serde_json::to_value(body).map_err(|error| ToolError::Api(ApiError::internal(error)))
}
