use std::io;

pub(crate) fn send_signal(process_id: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    if unsafe { libc::kill(process_id, signal) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub(crate) fn send_group_signal(process_group: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    send_signal(-process_group, signal)
}

pub(crate) fn try_wait_child(process_id: libc::pid_t) -> io::Result<Option<(libc::pid_t, i32)>> {
    let mut status = 0;
    let reaped = unsafe { libc::waitpid(process_id, &mut status, libc::WNOHANG) };
    if reaped == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((reaped != 0).then_some((reaped, status)))
}

pub(crate) fn install_signal_handler(
    signal: libc::c_int,
    handler: extern "C" fn(libc::c_int),
) -> io::Result<()> {
    let result = unsafe {
        let mut action: libc::sigaction = std::mem::zeroed();
        action.sa_sigaction = handler as *const () as usize;
        action.sa_flags = 0;
        libc::sigemptyset(&mut action.sa_mask);
        libc::sigaction(signal, &action, std::ptr::null_mut())
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn raise_signal(signal: libc::c_int) -> io::Result<()> {
    if unsafe { libc::raise(signal) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
