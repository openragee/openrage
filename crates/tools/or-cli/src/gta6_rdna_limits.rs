//! The memory a scan's workers may take: a ceiling a worker puts on itself, the most it has held,
//! and how many of them a machine's memory allows side by side.

/// The memory a worker may commit where no `--worker-memory` says, in megabytes.
pub(crate) const WORKER_MB: u64 = 1024;
/// The workers of a machine whose memory cannot be asked.
const WORKERS_UNMEASURED: usize = 4;

#[cfg(windows)]
mod os {
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};

    #[repr(C)]
    struct BasicLimits {
        per_process_user_time: i64,
        per_job_user_time: i64,
        limit_flags: u32,
        minimum_working_set: usize,
        maximum_working_set: usize,
        active_process_limit: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    struct ExtendedLimits {
        basic: BasicLimits,
        io: [u64; 6],
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    #[repr(C)]
    struct MemoryCounters {
        size: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        pools: [usize; 4],
        pagefile: usize,
        peak_pagefile: usize,
    }

    #[repr(C)]
    struct MemoryStatus {
        length: u32,
        load: u32,
        total_physical: u64,
        rest: [u64; 6],
    }

    const EXTENDED_LIMIT_INFORMATION: i32 = 9;
    const LIMIT_PROCESS_MEMORY: u32 = 0x100;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> *mut c_void;
        fn SetInformationJobObject(
            job: *mut c_void,
            class: i32,
            information: *const c_void,
            length: u32,
        ) -> i32;
        fn AssignProcessToJobObject(job: *mut c_void, process: *mut c_void) -> i32;
        fn GetCurrentProcess() -> *mut c_void;
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut MemoryCounters,
            size: u32,
        ) -> i32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
    }

    pub(super) fn limit_memory(bytes: u64) -> bool {
        // SAFETY: the structure is the one the class names, zeroed but for the limit, and read for
        // its stated length; the job's handle is kept open for the life of the process, which is
        // what keeps the limit on it.
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return false;
            }
            let mut limits: ExtendedLimits = zeroed();
            limits.basic.limit_flags = LIMIT_PROCESS_MEMORY;
            limits.process_memory_limit = bytes as usize;
            SetInformationJobObject(
                job,
                EXTENDED_LIMIT_INFORMATION,
                (&raw const limits).cast(),
                size_of::<ExtendedLimits>() as u32,
            ) != 0
                && AssignProcessToJobObject(job, GetCurrentProcess()) != 0
        }
    }

    pub(super) fn peak_bytes() -> u64 {
        // SAFETY: the counters are written for the length the call is given, which is theirs.
        unsafe {
            let mut counters: MemoryCounters = zeroed();
            counters.size = size_of::<MemoryCounters>() as u32;
            match K32GetProcessMemoryInfo(GetCurrentProcess(), &raw mut counters, counters.size) {
                0 => 0,
                _ => counters.peak_pagefile as u64,
            }
        }
    }

    pub(super) fn physical_bytes() -> Option<u64> {
        // SAFETY: the status is written for the length it states, which is its own.
        unsafe {
            let mut status: MemoryStatus = zeroed();
            status.length = size_of::<MemoryStatus>() as u32;
            (GlobalMemoryStatusEx(&raw mut status) != 0).then_some(status.total_physical)
        }
    }
}

// ponytail: no ceiling and no measure off Windows; `setrlimit` and `getrusage` are the upgrade,
// and until then the worker count there is the small fixed one.
#[cfg(not(windows))]
mod os {
    pub(super) fn limit_memory(_bytes: u64) -> bool {
        false
    }
    pub(super) fn peak_bytes() -> u64 {
        0
    }
    pub(super) fn physical_bytes() -> Option<u64> {
        None
    }
}

/// Holds this process to `megabytes` of committed memory: an allocation past it fails, which
/// ends the process with a word the scan reads. Whether the ceiling is on.
pub(crate) fn limit_memory(megabytes: u64) -> bool {
    os::limit_memory(megabytes << 20)
}

/// The most memory this process has committed so far, in megabytes; 0 where it cannot be asked.
pub(crate) fn peak_mb() -> u64 {
    os::peak_bytes() >> 20
}

/// The machine's physical memory in megabytes, where it can be asked.
pub(crate) fn physical_mb() -> Option<u64> {
    os::physical_bytes().map(|bytes| bytes >> 20)
}

/// How many workers run side by side: `asked`, else half the threads, since the machine is
/// shared; and never more than fit `budget_mb` at `worker_mb` each. Without a budget the
/// machine's is a quarter of its memory.
pub(crate) fn workers(
    asked: Option<usize>,
    threads: usize,
    budget_mb: Option<u64>,
    worker_mb: u64,
    physical_mb: Option<u64>,
) -> usize {
    let by_memory = match budget_mb.or(physical_mb.map(|all| all / 4)) {
        Some(budget) => (budget / worker_mb.max(1)) as usize,
        None => WORKERS_UNMEASURED,
    };
    asked.unwrap_or(threads / 2).min(by_memory).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workers_are_held_by_memory_as_well_as_by_threads() {
        let gib = 1024;
        // Half the threads where memory allows: a quarter of 64 GiB holds sixteen of 1 GiB.
        assert_eq!(workers(None, 32, None, gib, Some(64 * gib)), 16);
        assert_eq!(workers(None, 32, None, 2 * gib, Some(64 * gib)), 8);
        assert_eq!(workers(None, 8, None, gib, Some(64 * gib)), 4);
        // What is asked for is held by the budget too, and a budget stated replaces the machine's.
        assert_eq!(workers(Some(32), 32, None, gib, Some(64 * gib)), 16);
        assert_eq!(
            workers(Some(32), 32, Some(40 * gib), gib, Some(64 * gib)),
            32
        );
        assert_eq!(workers(Some(3), 32, None, gib, Some(64 * gib)), 3);
        // A machine that cannot be asked gets few, and no machine gets none.
        assert_eq!(workers(None, 32, None, gib, None), WORKERS_UNMEASURED);
        assert_eq!(workers(None, 1, Some(100), gib, None), 1);
    }

    #[cfg(windows)]
    #[test]
    fn this_machine_states_its_memory_and_this_process_its_peak() {
        assert!(physical_mb().is_some_and(|all| all > 0));
        assert!(os::peak_bytes() > 0);
    }
}
