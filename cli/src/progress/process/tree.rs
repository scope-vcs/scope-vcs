use std::{io, process::Child};

#[cfg(unix)]
pub(super) struct ProcessTree {
    process_group: libc::pid_t,
}

#[cfg(unix)]
impl ProcessTree {
    pub(super) fn attach(child: &Child) -> io::Result<Self> {
        let process_group = libc::pid_t::try_from(child.id()).map_err(io::Error::other)?;
        Ok(Self { process_group })
    }

    pub(super) fn terminate(&self) {
        unsafe {
            libc::kill(-self.process_group, libc::SIGKILL);
        }
    }
}

#[cfg(windows)]
pub(super) struct ProcessTree {
    job: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl ProcessTree {
    pub(super) fn attach(child: &Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject,
        };

        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        };
        let assigned = configured != 0
            && unsafe { AssignProcessToJobObject(job, child.as_raw_handle() as _) } != 0;
        if !assigned {
            let error = io::Error::last_os_error();
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(job);
            }
            return Err(error);
        }
        Ok(Self { job })
    }

    pub(super) fn terminate(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for ProcessTree {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}

#[cfg(not(any(unix, windows)))]
pub(super) struct ProcessTree;

#[cfg(not(any(unix, windows)))]
impl ProcessTree {
    pub(super) fn attach(_child: &Child) -> io::Result<Self> {
        Ok(Self)
    }

    pub(super) fn terminate(&self) {}
}
