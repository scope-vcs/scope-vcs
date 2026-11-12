mod git;
mod responses;
mod temp_dir;

pub(crate) use git::{commit_all, run_git};
pub(crate) use responses::repository_response;
use std::process::Output;
pub(crate) use temp_dir::TempDir;

impl TempDir {
    pub(crate) fn git_repo(label: &str, branch: &str) -> Self {
        let dir = Self::new(label);
        dir.run_git(["init", "--quiet", "-b", branch]);
        dir
    }

    pub(crate) fn run_git<const N: usize>(&self, args: [&str; N]) -> Output {
        run_git(self.path(), args)
    }
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
