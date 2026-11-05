use anyhow::{Context as _, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};

static SERVER: OnceLock<anyhow::Result<ProcessServer>> = OnceLock::new();

pub(super) fn database_url() -> anyhow::Result<String> {
    SERVER
        .get_or_init(ProcessServer::start)
        .as_ref()
        .map(|server| server.database_url.clone())
        .map_err(|error| anyhow::anyhow!("{error:#}"))
}

struct ProcessServer {
    root: PathBuf,
    bin: PathBuf,
    database_url: String,
}

impl ProcessServer {
    fn start() -> anyhow::Result<Self> {
        let bin = match std::env::var_os("SCOPE_TEST_POSTGRES_BIN") {
            Some(bin) => PathBuf::from(bin),
            None => PathBuf::from(run(Command::new("pg_config").arg("--bindir"))?.trim()),
        };
        let mut random = [0_u8; 8];
        getrandom::fill(&mut random).map_err(|error| anyhow::anyhow!("{error}"))?;
        let root = std::env::temp_dir().join(format!(
            "scope-pg-{}-{}",
            std::process::id(),
            hex::encode(random)
        ));
        let mut directory = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt as _;
            directory.mode(0o700);
        }
        directory.create(&root)?;
        let mut server = Self {
            root,
            bin,
            database_url: String::new(),
        };
        server.initialize()?;
        // SAFETY: the callback has C linkage, no arguments, and remains valid until process exit.
        let registered = unsafe { libc::atexit(stop_server_at_exit) };
        if registered != 0 {
            bail!("registering PostgreSQL test server shutdown failed with {registered}");
        }
        Ok(server)
    }

    fn initialize(&mut self) -> anyhow::Result<()> {
        run(Command::new(self.bin.join("initdb"))
            .args([
                "-U",
                "postgres",
                "--auth=trust",
                "--no-locale",
                "-E",
                "UTF8",
                "-D",
            ])
            .arg(self.root.join("data")))?;
        let socket = self.root.join("socket");
        fs::create_dir(&socket)?;
        fs::write(
            self.root.join("data/postgresql.auto.conf"),
            format!(
                "listen_addresses = ''\nport = 5432\nunix_socket_directories = '{}'\nmax_connections = 100\nsuperuser_reserved_connections = 3\nfsync = off\nfull_page_writes = off\n",
                socket.display().to_string().replace('\'', "''")
            ),
        )?;
        run(Command::new(self.bin.join("pg_ctl"))
            .arg("-D")
            .arg(self.root.join("data"))
            .arg("-l")
            .arg(self.root.join("postgres.log"))
            .args(["-w", "start"]))?;
        run(Command::new(self.bin.join("psql"))
            .arg("-h")
            .arg(&socket)
            .args([
                "-X",
                "-w",
                "-p",
                "5432",
                "-U",
                "postgres",
                "-d",
                "postgres",
                "-v",
                "ON_ERROR_STOP=1",
            ])
            .args(["-c", "CREATE ROLE scope LOGIN CREATEDB"])
            .args(["-c", "CREATE ROLE scope_blocked LOGIN CONNECTION LIMIT 0"])
            .args(["-c", "CREATE DATABASE scope_test OWNER scope"]))?;
        let mut url = url::Url::parse("postgres://scope@localhost:5432/scope_test")?;
        url.query_pairs_mut().append_pair(
            "host",
            socket
                .to_str()
                .context("PostgreSQL socket path is not UTF-8")?,
        );
        self.database_url = url.into();
        Ok(())
    }

    fn stop(&self) -> anyhow::Result<()> {
        if self.root.join("data/postmaster.pid").exists() {
            run(Command::new(self.bin.join("pg_ctl"))
                .arg("-D")
                .arg(self.root.join("data"))
                .args(["-w", "-m", "immediate", "stop"]))?;
        }
        fs::remove_dir_all(&self.root)?;
        Ok(())
    }
}

impl Drop for ProcessServer {
    fn drop(&mut self) {
        if let Err(error) = self.stop() {
            eprintln!("PostgreSQL test server shutdown failed: {error:#}");
        }
    }
}

extern "C" fn stop_server_at_exit() {
    if let Some(Ok(server)) = SERVER.get()
        && let Err(error) = server.stop()
    {
        eprintln!("PostgreSQL test server shutdown failed: {error:#}");
    }
}

fn run(command: &mut Command) -> anyhow::Result<String> {
    command
        .env_remove("PGHOSTADDR")
        .env_remove("PGSERVICE")
        .env_remove("PGOPTIONS");
    let output = command.output().with_context(|| {
        format!(
            "running {}; install PostgreSQL server tools or set SCOPE_TEST_POSTGRES_BIN",
            Path::new(command.get_program()).display()
        )
    })?;
    if !output.status.success() {
        bail!(
            "{} failed ({}): {}{}",
            Path::new(command.get_program()).display(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}
