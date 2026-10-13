use scope_api_contract::{
    RepoLifecycleState, RepoSummaryResponse, RepositoryAccessResponse, RepositoryActor,
};
use serde_json::Value;
use std::{
    path::Path,
    process::{Command, Output},
};

pub struct TempDir(tempfile::TempDir);

impl TempDir {
    pub fn new(label: &str) -> Self {
        Self(
            tempfile::Builder::new()
                .prefix(&format!("scope-cli-{label}-"))
                .tempdir()
                .unwrap(),
        )
    }

    pub fn git_repo(label: &str, branch: &str) -> Self {
        let dir = Self::new(label);
        dir.run_git(["init", "--quiet", "-b", branch]);
        dir
    }

    pub fn path(&self) -> &Path {
        self.0.path()
    }

    pub fn run_git<const N: usize>(&self, args: [&str; N]) -> Output {
        run_git(self.path(), args)
    }
}

pub fn run_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Output {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub fn commit_all(cwd: &Path, message: &str) -> String {
    run_git(cwd, ["add", "-A"]);
    run_git(
        cwd,
        [
            "-c",
            "user.name=Scope Test",
            "-c",
            "user.email=scope@example.test",
            "commit",
            "-qm",
            message,
        ],
    );
    String::from_utf8(run_git(cwd, ["rev-parse", "HEAD"]).stdout)
        .unwrap()
        .trim()
        .to_owned()
}

pub(crate) fn read_http_request(stream: &mut std::net::TcpStream) -> std::io::Result<String> {
    use std::io::{BufRead, Read};
    const MAX_HEADERS: usize = 64 * 1024;
    const MAX_BODY: usize = 1024 * 1024;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
    let mut reader = std::io::BufReader::new(stream);
    let mut request = Vec::new();
    let mut content_length = 0;
    loop {
        let start = request.len();
        let count = reader
            .by_ref()
            .take((MAX_HEADERS + 1 - start) as u64)
            .read_until(b'\n', &mut request)?;
        if count == 0 || request.len() > MAX_HEADERS {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "incomplete or oversized HTTP headers",
            ));
        }
        let line = std::str::from_utf8(&request[start..]).map_err(std::io::Error::other)?;
        if line == "\r\n" {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                content_length = value
                    .trim()
                    .parse::<usize>()
                    .map_err(std::io::Error::other)?;
            }
            if name.eq_ignore_ascii_case("transfer-encoding") {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "fixture requires Content-Length framing",
                ));
            }
        }
    }
    if content_length > MAX_BODY {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "oversized HTTP body",
        ));
    }
    let header_len = request.len();
    request.resize(header_len + content_length, 0);
    reader.read_exact(&mut request[header_len..])?;
    String::from_utf8(request).map_err(std::io::Error::other)
}

pub fn repository_response(overrides: Value) -> Value {
    let mut repo = serde_json::to_value(RepoSummaryResponse {
        description: None,
        website_url: None,
        id: "repo_one".into(),
        owner_handle: "owner".into(),
        name: "repo".into(),
        git_remote_url: "https://scope.example/git/public/owner/repo".into(),
        lifecycle_state: RepoLifecycleState::Ready,
        change_version: 1,
        content_version: 1,
        access: RepositoryAccessResponse {
            actor: RepositoryActor::Public,
            view: scope_api_contract::ViewId::public(),
            can_push: false,
            can_change_file_visibility: false,
            can_manage_members: false,
            can_delete_repo: false,
        },
        views: scope_api_contract::view_definitions(&scope_domain::views::Views::builtin()),
        open_request_count: 0,
    })
    .unwrap();
    merge(&mut repo, overrides);
    repo
}

fn merge(value: &mut Value, overrides: Value) {
    if let (Some(value), Some(overrides)) = (value.as_object_mut(), overrides.as_object()) {
        for (key, replacement) in overrides {
            merge(
                value.entry(key.clone()).or_insert(Value::Null),
                replacement.clone(),
            );
        }
    } else {
        *value = overrides;
    }
}
