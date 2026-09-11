use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

pub(crate) struct TestDir {
    pub(crate) path: PathBuf,
}

impl TestDir {
    pub(crate) fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "scope-cli-{label}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self { path }
    }

    pub(crate) fn git_repo(label: &str, branch: &str) -> Self {
        let dir = Self::new(label);
        let status = Command::new("git")
            .current_dir(&dir.path)
            .args(["init", "--quiet", "-b", branch])
            .status()
            .unwrap();
        assert!(status.success());
        dir
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn run_git<const N: usize>(&self, args: [&str; N]) -> Output {
        let output = Command::new("git")
            .current_dir(&self.path)
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
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Capture one bounded HTTP request, including its declared body, before replying.
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

#[test]
fn request_capture_waits_for_fragmented_body() {
    use std::{
        io::Write,
        net::{TcpListener, TcpStream},
        sync::mpsc,
        time::Duration,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (captured, received) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        captured
            .send(read_http_request(&mut stream).unwrap())
            .unwrap();
    });
    let mut client = TcpStream::connect(address).unwrap();
    client
        .write_all(b"POST / HTTP/1.1\r\nContent-Length: 5\r\n\r\n")
        .unwrap();
    assert!(received.recv_timeout(Duration::from_millis(30)).is_err());
    client.write_all(b"he").unwrap();
    client.write_all(b"llo").unwrap();
    assert!(
        received
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .ends_with("\r\n\r\nhello")
    );
    thread.join().unwrap();
}
