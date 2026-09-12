use crate::{api::api_url, auth::read_stored_session_token};
use anyhow::Context;
use std::io::{self, BufRead, Write};

#[derive(Debug, Default, Eq, PartialEq)]
struct GitCredentialRequest {
    protocol: Option<String>,
    host: Option<String>,
    path: Option<String>,
}

pub fn run_git_credential(operation: &str) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    write_git_credential_response(operation, stdin.lock(), stdout.lock())
}

fn write_git_credential_response(
    operation: &str,
    reader: impl BufRead,
    writer: impl Write,
) -> anyhow::Result<()> {
    let configured_api_url = api_url();
    write_git_credential_response_with(
        operation,
        reader,
        writer,
        &configured_api_url,
        read_stored_session_token,
    )
}

fn write_git_credential_response_with(
    operation: &str,
    reader: impl BufRead,
    mut writer: impl Write,
    configured_api_url: &str,
    read_token: impl FnOnce(&str) -> anyhow::Result<Option<String>>,
) -> anyhow::Result<()> {
    let request = parse_git_credential_request(reader)?;
    if operation != "get" {
        return Ok(());
    }

    if !is_scope_permissioned_credential_request(&request) {
        return Ok(());
    }
    let Some(session_token) = read_token(configured_api_url.trim_end_matches('/'))? else {
        return Ok(());
    };

    writeln!(writer, "username=scope").context("write Git credential username")?;
    writeln!(writer, "password={session_token}").context("write Git credential password")?;
    writeln!(writer).context("finish Git credential response")?;
    Ok(())
}

fn parse_git_credential_request(reader: impl BufRead) -> anyhow::Result<GitCredentialRequest> {
    let mut request = GitCredentialRequest::default();
    for line in reader.lines() {
        let line = line.context("read Git credential request")?;
        let line = line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "protocol" => request.protocol = Some(value.to_string()),
            "host" => request.host = Some(value.to_string()),
            "path" => request.path = Some(value.to_string()),
            _ => {}
        }
    }
    Ok(request)
}

fn is_scope_permissioned_credential_request(request: &GitCredentialRequest) -> bool {
    if !matches!(request.protocol.as_deref(), Some("http" | "https")) {
        return false;
    }
    if request
        .host
        .as_deref()
        .is_none_or(|host| host.trim().is_empty())
    {
        return false;
    }
    let Some(path) = request.path.as_deref() else {
        return false;
    };
    let path = format!("/{}", path.trim_start_matches('/'));
    let Some((_, repo_path)) = path.split_once("/git/permissioned/") else {
        return false;
    };
    repo_path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .take(2)
        .count()
        == 2
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn parse_git_credential_request_reads_scope_fields() {
        let request = parse_git_credential_request(Cursor::new(
            "protocol=https\nhost=scope.example\npath=git/permissioned/adam/repo\n\n",
        ))
        .unwrap();

        assert_eq!(
            request,
            GitCredentialRequest {
                protocol: Some("https".to_string()),
                host: Some("scope.example".to_string()),
                path: Some("git/permissioned/adam/repo".to_string()),
            }
        );
    }

    #[test]
    fn permissioned_credentials_accept_a_separate_git_host() {
        let request = GitCredentialRequest {
            protocol: Some("https".to_string()),
            host: Some("scope.example:8443".to_string()),
            path: Some("api/git/permissioned/adam/repo".to_string()),
        };

        assert!(is_scope_permissioned_credential_request(&request));
    }

    #[test]
    fn permissioned_credentials_ignore_public_or_incomplete_paths() {
        for path in [
            "git/public/adam/repo",
            "git/permissioned/adam",
            "other/permissioned/adam/repo",
        ] {
            let request = GitCredentialRequest {
                protocol: Some("https".to_string()),
                host: Some("scope.example".to_string()),
                path: Some(path.to_string()),
            };
            assert!(!is_scope_permissioned_credential_request(&request));
        }
    }

    #[test]
    fn write_git_credential_response_ignores_non_get_operations() {
        let mut output = Vec::new();
        write_git_credential_response(
            "store",
            Cursor::new("protocol=https\nhost=scope.example\npath=git/permissioned/adam/repo\n\n"),
            &mut output,
        )
        .unwrap();

        assert!(output.is_empty());
    }

    #[test]
    fn get_returns_the_session_for_the_configured_api_url() {
        let mut output = Vec::new();
        write_git_credential_response_with(
            "get",
            Cursor::new(
                "protocol=https\nhost=git.scope.example\npath=git/permissioned/adam/repo\n\n",
            ),
            &mut output,
            "https://api.scope.example/",
            |api_url| {
                assert_eq!(api_url, "https://api.scope.example");
                Ok(Some("scope_cli_secret".to_string()))
            },
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(output).unwrap(),
            "username=scope\npassword=scope_cli_secret\n\n"
        );
    }
}
