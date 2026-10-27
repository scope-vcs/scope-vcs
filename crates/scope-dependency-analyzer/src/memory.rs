use std::io;

#[cfg(unix)]
pub(crate) fn limit(bytes: u64) -> io::Result<()> {
    let mut current = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: getrlimit writes one rlimit value into the valid, writable `current`.
    if unsafe { libc::getrlimit(libc::RLIMIT_DATA, &mut current) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let bytes = libc::rlim_t::try_from(bytes).unwrap_or(libc::RLIM_INFINITY);
    let limit = libc::rlimit {
        rlim_cur: bytes.min(current.rlim_max),
        rlim_max: bytes.min(current.rlim_max),
    };
    // SAFETY: setrlimit reads one valid rlimit value and only changes this process's limits.
    if unsafe { libc::setrlimit(libc::RLIMIT_DATA, &limit) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn limit(bytes: u64) -> io::Result<()> {
    use windows_sys::Win32::System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        },
        Threading::GetCurrentProcess,
    };

    // SAFETY: CreateJobObjectW accepts null security attributes and a null name.
    let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if job.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.ProcessMemoryLimit = usize::try_from(bytes).unwrap_or(usize::MAX);
    // SAFETY: `job` is a live job handle and `limits` is a fully initialized
    // JOBOBJECT_EXTENDED_LIMIT_INFORMATION of the size passed.
    let configured = unsafe {
        SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            std::ptr::from_ref(&limits).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    };
    // SAFETY: `job` is a live job handle and GetCurrentProcess returns this process's pseudo-handle.
    if configured == 0 || unsafe { AssignProcessToJobObject(job, GetCurrentProcess()) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
