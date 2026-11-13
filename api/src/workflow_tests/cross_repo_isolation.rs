mod repo_a;
mod seeds;
mod write_bodies;

use super::custom_views::{MEMBER_SUBJECT, member_bearer};
use super::*;
use futures_util::StreamExt;
use repo_a::{OUTSIDER_EMAIL, OUTSIDER_SUBJECT, RepoA, VISIBLE_PATH, repo_a};
use scope_api_contract::routes::{self, ROUTE_TEMPLATES, path_segment};

const METHODS: [&str; 5] = ["GET", "POST", "PUT", "PATCH", "DELETE"];
const REFUSALS: [StatusCode; 7] = [
    StatusCode::BAD_REQUEST,
    StatusCode::UNAUTHORIZED,
    StatusCode::FORBIDDEN,
    StatusCode::NOT_FOUND,
    StatusCode::METHOD_NOT_ALLOWED,
    StatusCode::UNSUPPORTED_MEDIA_TYPE,
    StatusCode::UNPROCESSABLE_ENTITY,
];
const ACCESS_REFUSALS: [StatusCode; 4] = [
    StatusCode::UNAUTHORIZED,
    StatusCode::FORBIDDEN,
    StatusCode::NOT_FOUND,
    StatusCode::METHOD_NOT_ALLOWED,
];
const CALLER_SCOPED_SUCCESSES: &[(&str, &str)] = &[
    ("GET", routes::HEALTH),
    ("GET", routes::READINESS),
    ("GET", routes::MCP_PROTECTED_RESOURCE_METADATA),
    ("GET", routes::ACCOUNT_SESSION),
    ("GET", routes::CLI_SESSIONS),
    ("GET", routes::OWNER_REPOSITORIES),
    ("GET", routes::REPOSITORY_INVITE),
    ("POST", routes::CLI_DEVICE_LOGIN),
    ("POST", routes::CLI_EXCHANGE_GRANTS),
    ("POST", routes::REPOS),
    ("POST", routes::CLI_BROWSER_LOGIN),
    ("DELETE", routes::ACCOUNT),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Caller {
    Anonymous,
    Outsider,
    NarrowMember,
    Owner,
}

impl Caller {
    fn bearer(self) -> Option<String> {
        match self {
            Caller::Anonymous => None,
            Caller::Outsider => Some(bearer_header_for(OUTSIDER_SUBJECT, OUTSIDER_EMAIL)),
            Caller::NarrowMember => Some(member_bearer()),
            Caller::Owner => Some(bearer_header()),
        }
    }

    fn agent(self) -> Option<(&'static str, &'static str)> {
        match self {
            Caller::Anonymous => None,
            Caller::Outsider => Some(("oat_isolation_outsider", OUTSIDER_SUBJECT)),
            Caller::NarrowMember => Some(("oat_isolation_member", MEMBER_SUBJECT)),
            Caller::Owner => Some(("oat_isolation_owner", TEST_CLERK_USER_ID)),
        }
    }
}

struct Case {
    method: &'static str,
    template: &'static str,
    query: String,
}

impl Case {
    fn ends_caller_account(&self) -> bool {
        self.method == "DELETE" && self.template == routes::ACCOUNT
    }

    fn may_succeed_for_outsiders(&self) -> bool {
        CALLER_SCOPED_SUCCESSES.contains(&(self.method, self.template))
    }

    fn names_repository_children(&self) -> bool {
        let repository_level = ["{owner}", "{repo}", "{view}"]
            .iter()
            .filter(|parameter| self.template.contains(**parameter))
            .count();
        self.template.contains("{owner}/{repo}")
            && self.template.matches('{').count() > repository_level
    }

    fn uri(&self, params: &BTreeMap<&'static str, String>) -> String {
        let mut uri = String::new();
        let mut rest = self.template;
        while let Some(start) = rest.find('{') {
            let end = start + rest[start..].find('}').unwrap();
            let name = &rest[start + 1..end];
            let value = params.get(name).unwrap_or_else(|| {
                panic!(
                    "{}: the isolation fixture has no value for {{{name}}}; map it in repo_a()",
                    self.template
                )
            });
            uri.push_str(&rest[..start]);
            uri.push_str(&path_segment(value));
            rest = &rest[end + 1..];
        }
        uri.push_str(rest);
        if !self.query.is_empty() {
            uri.push('?');
            uri.push_str(&self.query);
        }
        uri
    }
}

struct Observed {
    status: StatusCode,
    git_error: bool,
    tool_error: bool,
    text: String,
}

impl Observed {
    fn refused(&self, method: &str) -> bool {
        let refusals: &[StatusCode] = if method == "GET" {
            &REFUSALS
        } else {
            &ACCESS_REFUSALS
        };
        refusals.contains(&self.status) || self.git_error || self.tool_error
    }
}

fn cases(repo_a: &RepoA) -> Vec<Case> {
    let mut cases = Vec::new();
    let write_queries = [repo_a.write_query.clone()];
    for method in METHODS {
        for template in ROUTE_TEMPLATES {
            let queries = if method == "GET" {
                &repo_a.queries[..]
            } else {
                &write_queries[..]
            };
            for query in queries {
                cases.push(Case {
                    method,
                    template,
                    query: query.clone(),
                });
            }
        }
    }
    cases.sort_by_key(Case::ends_caller_account);
    cases
}

async fn call(repo_a: &RepoA, case: &Case, uri: &str, caller: Caller) -> Observed {
    let body = (case.method != "GET").then(|| {
        repo_a
            .write_bodies
            .get(&(case.method, case.template))
            .map_or("{}", String::as_str)
    });
    let bearer = if case.template == routes::MCP {
        caller.agent().map(|(token, _)| format!("Bearer {token}"))
    } else {
        caller.bearer()
    };
    let response = api_request(
        repo_a.app.clone(),
        case.method,
        uri,
        bearer.as_deref(),
        body,
    )
    .await;
    let status = response.status();
    let mut text = format!("{:?}", response.headers());
    let content_type = response
        .headers()
        .get(CONTENT_TYPE)
        .map(|value| value.as_bytes().to_vec())
        .unwrap_or_default();
    let streams = content_type.starts_with(b"text/event-stream");
    let git = content_type.starts_with(b"application/x-git-");
    let mut git_error = false;
    let mut tool_error = false;
    if streams && status.is_success() {
        let mut events = response.into_body().into_data_stream();
        let first = events.next().await.unwrap().unwrap();
        text.push_str(&String::from_utf8_lossy(&first));
    } else {
        let body = to_bytes(response.into_body(), 16 * 1024 * 1024)
            .await
            .unwrap();
        git_error = git && body.get(4..8) == Some(b"ERR ".as_slice());
        tool_error = case.template == routes::MCP
            && serde_json::from_slice::<serde_json::Value>(&body)
                .is_ok_and(|reply| reply["result"]["isError"] == true);
        text.push_str(&String::from_utf8_lossy(&body));
    }
    Observed {
        status,
        git_error,
        tool_error,
        text,
    }
}

fn leaked<'a>(text: &str, markers: &'a [String]) -> Vec<&'a str> {
    markers
        .iter()
        .filter(|marker| text.contains(marker.as_str()))
        .map(String::as_str)
        .collect()
}

fn normalized(text: &str, values: &BTreeMap<&'static str, String>) -> String {
    let repository = format!("{}/{}", values["owner"], values["repo"]);
    let mut text = replace_word(text, &repository, "{owner}/{repo}");
    let mut identifiers = values
        .iter()
        .filter(|(_, value)| value.len() >= 8)
        .collect::<Vec<_>>();
    identifiers.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
    for (name, value) in identifiers {
        text = replace_word(&text, value, &format!("{{{name}}}"));
    }
    text
}

fn replace_word(text: &str, word: &str, replacement: &str) -> String {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find(word) {
        let before = rest[..index].chars().next_back();
        let after = rest[index + word.len()..].chars().next();
        out.push_str(&rest[..index]);
        if before.is_some_and(is_word) || after.is_some_and(is_word) {
            out.push_str(word);
        } else {
            out.push_str(replacement);
        }
        rest = &rest[index + word.len()..];
    }
    out.push_str(rest);
    out
}

struct Checked {
    caller: Caller,
    failures: Vec<String>,
    text: String,
}

async fn check(repo_a: &RepoA, case: &Case, caller: Caller) -> Checked {
    let uri = case.uri(&repo_a.params);
    let observed = call(repo_a, case, &uri, caller).await;
    let mut failures = Vec::new();
    let markers: &[String] = match caller {
        Caller::Owner => &[],
        Caller::NarrowMember => &repo_a.narrow_markers,
        Caller::Anonymous | Caller::Outsider => &repo_a.outsider_markers,
    };
    let leaks = leaked(&observed.text, markers);
    if !leaks.is_empty() {
        failures.push(format!(
            "{caller:?} {} {uri}: leaked {leaks:?}",
            case.method
        ));
    }
    let outsider = matches!(caller, Caller::Anonymous | Caller::Outsider);
    if outsider && !case.may_succeed_for_outsiders() {
        if !observed.refused(case.method) {
            failures.push(format!(
                "{caller:?} {} {uri}: {} is not a refusal: {}",
                case.method, observed.status, observed.text
            ));
        }
        if observed.status != StatusCode::METHOD_NOT_ALLOWED {
            let absent = call(repo_a, case, &case.uri(&repo_a.absent), caller).await;
            let observed_text = normalized(&observed.text, &repo_a.params);
            let absent_text = normalized(&absent.text, &repo_a.absent);
            if observed.status != absent.status || observed_text != absent_text {
                failures.push(format!(
                    "{caller:?} {} {uri}: repo A answers {} {observed_text} but an absent repo answers {} {absent_text}",
                    case.method, observed.status, absent.status
                ));
            }
        }
    }
    if caller == Caller::Outsider && case.names_repository_children() {
        failures.extend(children_through_own_repository(repo_a, case).await);
    }
    if caller == Caller::NarrowMember && case.template.contains("{view}") {
        let mut own_view = repo_a.params.clone();
        own_view.insert("view", "agent".to_string());
        let uri = case.uri(&own_view);
        let observed = call(repo_a, case, &uri, caller).await;
        let mut markers = repo_a.narrow_markers.clone();
        markers.push(repo_a.params["head_oid"].clone());
        let leaks = leaked(&observed.text, &markers);
        if !leaks.is_empty() {
            failures.push(format!(
                "{caller:?} {} {uri}: leaked {leaks:?}",
                case.method
            ));
        }
    }
    Checked {
        caller,
        failures,
        text: observed.text,
    }
}

async fn children_through_own_repository(repo_a: &RepoA, case: &Case) -> Vec<String> {
    let uri = case.uri(&repo_a.children_in_b);
    let observed = call(repo_a, case, &uri, Caller::Outsider).await;
    let absent = call(
        repo_a,
        case,
        &case.uri(&repo_a.absent_children_in_b),
        Caller::Outsider,
    )
    .await;
    let mut failures = Vec::new();
    let leaks = leaked(&observed.text, &repo_a.outsider_markers);
    if !leaks.is_empty() {
        failures.push(format!("Outsider {} {uri}: leaked {leaks:?}", case.method));
    }
    let observed_text = normalized(&observed.text, &repo_a.children_in_b);
    let absent_text = normalized(&absent.text, &repo_a.absent_children_in_b);
    if observed.status != absent.status || observed_text != absent_text {
        failures.push(format!(
            "Outsider {} {uri}: repo A's ids under repo B answer {} {observed_text} but absent ids answer {} {absent_text}",
            case.method, observed.status, absent.status
        ));
    }
    failures
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_route_returns_another_repositorys_data_or_a_hidden_views_files() {
    let repo_a = repo_a().await;
    let cases = cases(&repo_a);
    let (reads, writes): (Vec<_>, Vec<_>) = cases.iter().partition(|case| case.method == "GET");
    let callers = [
        Caller::Owner,
        Caller::Anonymous,
        Caller::Outsider,
        Caller::NarrowMember,
    ];
    let mut checked = futures_util::stream::iter(
        reads
            .iter()
            .flat_map(|case| callers.map(|caller| check(&repo_a, case, caller))),
    )
    .buffer_unordered(8)
    .collect::<Vec<_>>()
    .await;
    for caller in [Caller::Anonymous, Caller::Outsider, Caller::NarrowMember] {
        for case in &writes {
            checked.push(check(&repo_a, case, caller).await);
        }
    }

    let seen_by = |caller| {
        checked
            .iter()
            .filter(|checked| checked.caller == caller)
            .map(|checked| checked.text.as_str())
            .collect::<String>()
    };
    let owner_seen = seen_by(Caller::Owner);
    let unseen = repo_a
        .outsider_markers
        .iter()
        .filter(|marker| !owner_seen.contains(marker.as_str()))
        .collect::<Vec<_>>();
    let failures = checked
        .iter()
        .flat_map(|checked| checked.failures.iter().map(String::as_str))
        .collect::<Vec<_>>();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(unseen.is_empty(), "the owner never read {unseen:?}");
    assert!(seen_by(Caller::NarrowMember).contains(VISIBLE_PATH));
}
