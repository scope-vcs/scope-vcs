pub(super) fn register_exit_hook(hook: extern "C" fn()) -> Result<(), libc::c_int> {
    match unsafe { libc::atexit(hook) } {
        0 => Ok(()),
        code => Err(code),
    }
}

#[cfg(unix)]
pub(super) fn effective_user_id() -> libc::uid_t {
    unsafe { libc::geteuid() }
}

#[cfg(unix)]
pub(super) fn process_is_alive(pid: libc::pid_t) -> bool {
    let signalled = unsafe { libc::kill(pid, 0) == 0 };
    signalled || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}
