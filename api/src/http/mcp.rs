use super::origins::{public_app_origin, public_mcp_resource};
use crate::{
    auth::{
        agent::{AgentCaller, require_agent_caller},
        viewer::Viewer,
    },
    error::ApiError,
    state::AppState,
};
use axum::{
    Json,
    body::Bytes,
    extract::State,
    http::{
        HeaderMap, HeaderValue, StatusCode,
        header::{ORIGIN, WWW_AUTHENTICATE},
    },
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use scope_api_contract::routes;
use scope_domain::account::agent_access::AgentScope;
use serde::Deserialize;
use serde_json::{Value, json};

mod tools;

const MODERN_VERSIONS: [&str; 1] = ["2026-07-28"];
const LEGACY_VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
const PROTOCOL_VERSION_META: &str = "io.modelcontextprotocol/protocolVersion";
const CLIENT_CAPABILITIES_META: &str = "io.modelcontextprotocol/clientCapabilities";
const SERVER_INFO_META: &str = "io.modelcontextprotocol/serverInfo";
const PROTOCOL_VERSION_HEADER: &str = "mcp-protocol-version";
const METHOD_HEADER: &str = "mcp-method";
const NAME_HEADER: &str = "mcp-name";
const PARSE_ERROR: i64 = -32700;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const HEADER_MISMATCH: i64 = -32020;
const UNSUPPORTED_PROTOCOL_VERSION: i64 = -32022;
const CACHE_TTL_MS: u64 = 5 * 60 * 1000;
const INSTRUCTIONS: &str = "Read Scope repositories and contribution requests. Name a repository as owner/name. Identify a request by the req_ ID that request_list returns.";

#[derive(Deserialize)]
struct RpcRequest {
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy,
}

#[derive(Deserialize)]
struct ToolCall {
    name: String,
    #[serde(default)]
    arguments: Value,
}

pub(crate) async fn protected_resource_metadata(
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let resource = public_mcp_resource()?;
    let issuer = state.clerk.issuer.ok_or_else(|| {
        ApiError::infrastructure_unavailable("CLERK_ISSUER is required to serve agents")
    })?;
    Ok(Json(json!({
        "resource": resource,
        "resource_name": "Scope",
        "authorization_servers": [issuer],
        "scopes_supported": scope_keys(),
        "bearer_methods_supported": ["header"],
    })))
}

pub(crate) async fn serve(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(origin) = headers.get(ORIGIN)
        && public_app_origin("serve agents").ok().as_deref() != origin.to_str().ok()
    {
        return ApiError::forbidden("origin not allowed").into_response();
    }
    let request = match serde_json::from_slice::<RpcRequest>(&body) {
        Ok(request) => request,
        Err(error) => {
            return rpc_error(
                StatusCode::BAD_REQUEST,
                None,
                PARSE_ERROR,
                &format!("invalid JSON-RPC message: {error}"),
                None,
            );
        }
    };
    let Some(id) = request.id.clone() else {
        return StatusCode::ACCEPTED.into_response();
    };
    let era = match request_era(&headers, &request, &id) {
        Ok(era) => era,
        Err(response) => return *response,
    };
    let resource = match public_mcp_resource() {
        Ok(resource) => resource,
        Err(error) => return error.into_response(),
    };
    let caller = match require_agent_caller(&state, &headers, &resource).await {
        Ok(caller) => caller,
        Err(error) if error.status() == StatusCode::UNAUTHORIZED => {
            return challenged(error, &sign_in_challenge(&resource));
        }
        Err(error) => return error.into_response(),
    };
    match (era, request.method.as_str()) {
        (Era::Modern, "server/discover") => result(
            era,
            id,
            cacheable(json!({
                "supportedVersions": MODERN_VERSIONS,
                "capabilities": { "tools": {} },
                "instructions": INSTRUCTIONS,
            })),
        ),
        (Era::Legacy, "initialize") => result(
            era,
            id,
            json!({
                "protocolVersion": legacy_version(&request.params),
                "capabilities": { "tools": {} },
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS,
            }),
        ),
        (Era::Legacy, "ping") => result(era, id, json!({})),
        (_, "tools/list") => result(era, id, cacheable(json!({ "tools": tools::definitions() }))),
        (_, "tools/call") => call_tool(&state, era, id, request.params, caller, &resource).await,
        (Era::Modern, method) => rpc_error(
            StatusCode::NOT_FOUND,
            Some(id),
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
            None,
        ),
        (Era::Legacy, method) => rpc_error(
            StatusCode::OK,
            Some(id),
            METHOD_NOT_FOUND,
            &format!("method not found: {method}"),
            None,
        ),
    }
}

async fn call_tool(
    state: &AppState,
    era: Era,
    id: Value,
    params: Value,
    caller: AgentCaller,
    resource: &str,
) -> Response {
    let call = match serde_json::from_value::<ToolCall>(params) {
        Ok(call) => call,
        Err(error) => {
            return rpc_error(
                StatusCode::OK,
                Some(id),
                INVALID_PARAMS,
                &format!("invalid tools/call params: {error}"),
                None,
            );
        }
    };
    let Some(operation) = tools::Operation::named(&call.name) else {
        return rpc_error(
            StatusCode::OK,
            Some(id),
            INVALID_PARAMS,
            &format!("unknown tool: {}", call.name),
            None,
        );
    };
    let scope = operation.scope();
    if !caller.allows(scope) {
        let error = ApiError::forbidden(format!(
            "{} needs the {} permission; sign the agent in again to grant it",
            call.name,
            scope.key()
        ));
        return challenged(error, &insufficient_scope_challenge(resource, scope));
    }
    let outcome = operation
        .run(state, Viewer::signed_in(caller.user), call.arguments)
        .await;
    result(era, id, tools::call_result(outcome))
}

fn request_era(
    headers: &HeaderMap,
    request: &RpcRequest,
    id: &Value,
) -> Result<Era, Box<Response>> {
    let refuse = |code, message: &str, data| {
        Box::new(rpc_error(
            StatusCode::BAD_REQUEST,
            Some(id.clone()),
            code,
            message,
            data,
        ))
    };
    let meta = request.params.get("_meta");
    let version = meta
        .and_then(|meta| meta.get(PROTOCOL_VERSION_META))
        .and_then(Value::as_str);
    let Some(version) = version.filter(|_| request.method != "initialize") else {
        return Ok(Era::Legacy);
    };
    if !MODERN_VERSIONS.contains(&version) {
        let supported = MODERN_VERSIONS.iter().chain(LEGACY_VERSIONS.iter());
        return Err(refuse(
            UNSUPPORTED_PROTOCOL_VERSION,
            "unsupported protocol version",
            Some(json!({ "supported": supported.collect::<Vec<_>>(), "requested": version })),
        ));
    }
    if meta
        .and_then(|meta| meta.get(CLIENT_CAPABILITIES_META))
        .is_none_or(|capabilities| !capabilities.is_object())
    {
        return Err(refuse(
            INVALID_PARAMS,
            &format!("{CLIENT_CAPABILITIES_META} is required"),
            None,
        ));
    }
    let mut mirrored = vec![
        (PROTOCOL_VERSION_HEADER, Some(version)),
        (METHOD_HEADER, Some(request.method.as_str())),
    ];
    if request.method == "tools/call" {
        mirrored.push((
            NAME_HEADER,
            request.params.get("name").and_then(Value::as_str),
        ));
    }
    for (header, body) in mirrored {
        let sent = headers
            .get(header)
            .and_then(|value| value.to_str().ok())
            .map(decoded_header_value);
        if sent.is_none() || sent.as_deref() != body {
            return Err(refuse(
                HEADER_MISMATCH,
                &format!("header mismatch: {header} does not match the request body"),
                None,
            ));
        }
    }
    Ok(Era::Modern)
}

fn decoded_header_value(value: &str) -> String {
    value
        .strip_prefix("=?base64?")
        .and_then(|rest| rest.strip_suffix("?="))
        .and_then(|encoded| BASE64.decode(encoded).ok())
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| value.to_string())
}

fn legacy_version(params: &Value) -> &'static str {
    let requested = params.get("protocolVersion").and_then(Value::as_str);
    LEGACY_VERSIONS
        .into_iter()
        .find(|version| Some(*version) == requested)
        .unwrap_or(LEGACY_VERSIONS[0])
}

fn cacheable(mut result: Value) -> Value {
    result["ttlMs"] = json!(CACHE_TTL_MS);
    result["cacheScope"] = json!("public");
    result
}

fn result(era: Era, id: Value, mut result: Value) -> Response {
    if era == Era::Modern
        && let Some(fields) = result.as_object_mut()
    {
        fields.insert("resultType".into(), json!("complete"));
        fields.insert("_meta".into(), json!({ SERVER_INFO_META: server_info() }));
    }
    Json(json!({ "jsonrpc": "2.0", "id": id, "result": result })).into_response()
}

fn rpc_error(
    status: StatusCode,
    id: Option<Value>,
    code: i64,
    message: &str,
    data: Option<Value>,
) -> Response {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    let mut body = json!({ "jsonrpc": "2.0", "error": error });
    if let Some(id) = id {
        body["id"] = id;
    }
    (status, Json(body)).into_response()
}

fn challenged(error: ApiError, challenge: &str) -> Response {
    let mut response = error.into_response();
    if let Ok(value) = HeaderValue::from_str(challenge) {
        response.headers_mut().insert(WWW_AUTHENTICATE, value);
    }
    response
}

fn sign_in_challenge(resource: &str) -> String {
    format!(
        r#"Bearer resource_metadata="{}", scope="{}""#,
        metadata_url(resource),
        scope_keys().join(" ")
    )
}

fn insufficient_scope_challenge(resource: &str, scope: AgentScope) -> String {
    format!(
        r#"Bearer error="insufficient_scope", scope="{}", resource_metadata="{}""#,
        scope.key(),
        metadata_url(resource)
    )
}

fn metadata_url(resource: &str) -> String {
    let origin = resource.strip_suffix(routes::MCP).unwrap_or(resource);
    format!("{origin}{}", routes::MCP_PROTECTED_RESOURCE_METADATA)
}

fn scope_keys() -> Vec<&'static str> {
    AgentScope::ALL.into_iter().map(AgentScope::key).collect()
}

fn server_info() -> Value {
    json!({ "name": "scope", "title": "Scope", "version": env!("CARGO_PKG_VERSION") })
}
