use crate::posix;
use std::{
    collections::BTreeSet,
    fs,
    io::ErrorKind,
    process::{Child, Command},
    sync::atomic::{AtomicI32, Ordering},
    thread,
    time::{Duration, Instant},
};

const INTERNAL_REAPER_CHILD_ENV: &str = "SCOPE_INTERNAL_REAPER_CHILD";

static PENDING_REAPER_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
const REAPER_IDLE_POLL: Duration = Duration::from_millis(25);

pub fn install_pid1_reaper_if_needed() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;

    if std::process::id() != 1 || std::env::var_os(INTERNAL_REAPER_CHILD_ENV).is_some() {
        return Ok(());
    }

    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .args(std::env::args_os().skip(1))
        .env(INTERNAL_REAPER_CHILD_ENV, "1")
        .process_group(0);
    install_reaper_signal_handlers()?;
    let child = command.spawn()?;
    reap_service_process(child.id())
}

extern "C" fn remember_reaper_signal(signal: libc::c_int) {
    PENDING_REAPER_SIGNAL.store(signal, Ordering::SeqCst);
}

fn install_reaper_signal_handlers() -> std::io::Result<()> {
    for signal in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        posix::install_signal_handler(signal, remember_reaper_signal)?;
    }
    Ok(())
}

fn reap_service_process(service_pid: u32) -> std::io::Result<()> {
    let service_pid = i32::try_from(service_pid)
        .map_err(|_| std::io::Error::other("service process id exceeds i32"))?;
    loop {
        forward_pending_signal(service_pid);
        let Some((reaped, status)) = reap_exited_child()? else {
            thread::sleep(REAPER_IDLE_POLL);
            continue;
        };
        if reaped != service_pid {
            continue;
        }

        posix::send_group_signal(service_pid, libc::SIGKILL).ok();
        drain_adopted_descendants();
        std::process::exit(wait_status_exit_code(status));
    }
}

fn forward_pending_signal(service_pid: i32) {
    let signal = PENDING_REAPER_SIGNAL.swap(0, Ordering::SeqCst);
    if signal != 0 {
        posix::send_group_signal(service_pid, signal).ok();
    }
}

fn drain_adopted_descendants() {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        reap_exited_descendants();
        let descendants = child_process_ids().unwrap_or_default();
        if descendants.is_empty() {
            return;
        }

        for process_id in descendants {
            let Ok(process_id) = i32::try_from(process_id) else {
                continue;
            };
            posix::send_group_signal(process_id, libc::SIGKILL).ok();
            posix::send_signal(process_id, libc::SIGKILL).ok();
        }
        if Instant::now() >= deadline {
            return;
        }
        thread::sleep(Duration::from_millis(1));
    }
}

fn reap_exited_descendants() {
    while let Ok(Some(_)) = reap_exited_child() {}
}

#[cfg(unix)]
fn reap_exited_child() -> std::io::Result<Option<(i32, i32)>> {
    loop {
        match posix::try_wait_child(-1) {
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

#[cfg(test)]
#[cfg(unix)]
mod reaper_tests;

#[cfg(unix)]
pub(crate) fn wait_status_exit_code(status: i32) -> i32 {
    if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    }
}

fn parse_trimmed_usize(value: &str) -> Option<usize> {
    value.trim().parse().ok()
}

fn child_process_ids() -> Option<BTreeSet<usize>> {
    let mut children = BTreeSet::new();
    for task in fs::read_dir("/proc/self/task").ok()?.filter_map(Result::ok) {
        let contents = fs::read_to_string(task.path().join("children")).unwrap_or_default();
        children.extend(contents.split_whitespace().filter_map(parse_trimmed_usize));
    }
    Some(children)
}

#[cfg(test)]
pub(crate) fn child_process_state(pid: usize) -> Option<String> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let command_end = stat.rfind(')')?;
    stat.get(command_end + 2..)?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

pub fn configure_process_group(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    command.process_group(0);
}

pub fn kill_process_group(process_id: u32) {
    if let Ok(process_group) = i32::try_from(process_id) {
        posix::send_group_signal(process_group, libc::SIGKILL).ok();
    }
}

pub(crate) fn terminate_and_reap(child: &mut Child) {
    kill_process_group(child.id());
    child.kill().ok();
    child.wait().ok();
}

pub(crate) struct ChildGuard {
    child: Child,
    armed: bool,
}

impl ChildGuard {
    pub(crate) fn new(child: Child) -> Self {
        Self { child, armed: true }
    }

    pub(crate) fn disarm(&mut self) {
        self.armed = false;
    }

    pub(crate) fn terminate_and_reap(&mut self) {
        terminate_and_reap(&mut self.child);
        self.disarm();
    }
}

impl std::ops::Deref for ChildGuard {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.child
    }
}

impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.armed {
            self.terminate_and_reap();
        }
    }
}
