//! Process containment.
//!
//! On Windows each server runs inside a Job Object so that the whole process
//! tree is terminated together, memory limits are enforced by the OS, CPU
//! affinity is applied and — critically — the tree dies with WinPanel if the
//! panel itself crashes (KILL_ON_JOB_CLOSE).
//!
//! On Unix (used for development and for running the panel on a Linux box) a
//! new session is created and signals are sent to the process group.

use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Default)]
pub struct Limits {
    pub memory_mb: u64,
    pub cpu_limit_percent: u64,
    pub cpu_cores: Vec<usize>,
}

pub struct ProcessGuard {
    inner: imp::Inner,
    terminated: AtomicBool,
}

impl ProcessGuard {
    /// Attach containment to a freshly spawned process. Returns `None` when the
    /// platform refused (the caller still has PID-based termination fallbacks).
    pub fn attach(pid: u32, limits: &Limits) -> Option<ProcessGuard> {
        imp::attach(pid, limits).map(|inner| ProcessGuard {
            inner,
            terminated: AtomicBool::new(false),
        })
    }

    /// Immediately terminate the whole process tree.
    pub fn terminate(&self) {
        if self.terminated.swap(true, Ordering::SeqCst) {
            return;
        }
        imp::terminate(&self.inner);
    }

    /// Ask the process tree to exit (SIGTERM / console break where available).
    pub fn request_stop(&self) {
        imp::request_stop(&self.inner);
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        imp::release(&self.inner);
    }
}

#[cfg(windows)]
mod imp {
    use super::Limits;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectCpuRateControlInformation,
        JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_BASIC_LIMIT_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, SetProcessAffinityMask, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
        PROCESS_TERMINATE,
    };

    pub struct Inner {
        job: HANDLE,
        pid: u32,
    }

    // Job handles are kernel objects and are safe to use from any thread.
    unsafe impl Send for Inner {}
    unsafe impl Sync for Inner {}

    pub fn attach(pid: u32, limits: &Limits) -> Option<Inner> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                tracing::warn!("could not create job object for pid {pid}");
                return None;
            }

            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            let basic: &mut JOBOBJECT_BASIC_LIMIT_INFORMATION = &mut info.BasicLimitInformation;
            basic.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if limits.memory_mb > 0 {
                basic.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = (limits.memory_mb as usize).saturating_mul(1024 * 1024);
            }
            let set_limits = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );

            if limits.cpu_limit_percent > 0 && limits.cpu_limit_percent < 100 {
                // JOBOBJECT_CPU_RATE_CONTROL_INFORMATION is { u32 flags; u32 rate }
                // where rate is expressed in hundredths of a percent.
                let payload: [u32; 2] = [
                    JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                    (limits.cpu_limit_percent as u32) * 100,
                ];
                let ok = SetInformationJobObject(
                    job,
                    JobObjectCpuRateControlInformation,
                    payload.as_ptr() as *const c_void,
                    8,
                );
                if ok == 0 {
                    tracing::warn!("CPU rate control is not available for pid {pid}");
                }
            }

            let access = PROCESS_SET_QUOTA | PROCESS_TERMINATE | PROCESS_SET_INFORMATION;
            let process = OpenProcess(access, 0, pid);
            if process.is_null() {
                tracing::warn!("could not open process {pid} to attach the job object");
                CloseHandle(job);
                return None;
            }

            let assigned = AssignProcessToJobObject(job, process);
            CloseHandle(process);
            if assigned == 0 {
                // Typically caused by the panel itself running inside a job that
                // forbids nesting. Termination falls back to taskkill /T.
                tracing::warn!(
                    "could not assign pid {pid} to a job object; using taskkill fallback"
                );
                CloseHandle(job);
                return None;
            }
            if set_limits == 0 {
                tracing::warn!("job memory limits were rejected for pid {pid}");
            }

            if !limits.cpu_cores.is_empty() {
                let mut mask: usize = 0;
                for core in &limits.cpu_cores {
                    if *core < usize::BITS as usize {
                        mask |= 1usize << core;
                    }
                }
                if mask != 0 {
                    if let Some(process) = open_for_affinity(pid) {
                        SetProcessAffinityMask(process, mask);
                        CloseHandle(process);
                    }
                }
            }

            Some(Inner { job, pid })
        }
    }

    fn open_for_affinity(pid: u32) -> Option<HANDLE> {
        let handle = unsafe { OpenProcess(PROCESS_SET_INFORMATION, 0, pid) };
        if handle.is_null() {
            None
        } else {
            Some(handle)
        }
    }

    pub fn terminate(inner: &Inner) {
        unsafe {
            if !inner.job.is_null() {
                TerminateJobObject(inner.job, 1);
            }
        }
        kill_tree(inner.pid);
    }

    pub fn request_stop(inner: &Inner) {
        // No reliable console signal for windowless children: ask nicely via
        // taskkill (no /F) and let the supervisor escalate on timeout.
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &inner.pid.to_string(), "/T"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }

    pub fn release(inner: &Inner) {
        unsafe {
            if !inner.job.is_null() {
                CloseHandle(inner.job);
            }
        }
    }

    fn kill_tree(pid: u32) {
        let _ = std::process::Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(unix)]
mod imp {
    use super::Limits;

    pub struct Inner {
        pid: u32,
    }

    pub fn attach(pid: u32, limits: &Limits) -> Option<Inner> {
        if limits.memory_mb > 0 || limits.cpu_limit_percent > 0 || !limits.cpu_cores.is_empty() {
            tracing::debug!(
                "memory and CPU limits are enforced natively on Windows only (pid {pid})"
            );
        }
        Some(Inner { pid })
    }

    pub fn terminate(inner: &Inner) {
        unsafe {
            // The child was started with setsid(), so it leads its own group.
            let pid = inner.pid as i32;
            libc::kill(-pid, libc::SIGKILL);
            libc::kill(pid, libc::SIGKILL);
        }
    }

    pub fn request_stop(inner: &Inner) {
        unsafe {
            let pid = inner.pid as i32;
            libc::kill(-pid, libc::SIGTERM);
        }
    }

    pub fn release(_inner: &Inner) {}
}
