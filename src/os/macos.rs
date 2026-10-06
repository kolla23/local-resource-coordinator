//! The macOS versions of the memory and process-inspection functions of `sys`.

use crate::sys::ProcessInfo;

#[cfg(target_os = "macos")]
pub fn total_memory() -> u64 {
    sysctl_u64(b"hw.memsize\0").unwrap_or(0)
}

#[cfg(target_os = "macos")]
pub fn available_memory() -> u64 {
    vm_available().unwrap_or_else(total_memory)
}

#[cfg(target_os = "macos")]
fn sysctl_u64(name: &[u8]) -> Option<u64> {
    let mut value: u64 = 0;
    let mut len = std::mem::size_of::<u64>();
    let rc = unsafe {
        libc::sysctlbyname(
            name.as_ptr() as *const libc::c_char,
            &mut value as *mut u64 as *mut libc::c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    (rc == 0).then_some(value)
}

#[cfg(target_os = "macos")]
fn vm_available() -> Option<u64> {
    // The structure and the count come from `libc`. qex made its own structure
    // before, and that structure was WRONG: the real one mixes 32-bit and
    // 64-bit fields and it aligns to 8 bytes, so the size that qex sent to the
    // kernel did not agree with the size that the kernel writes.
    let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
    let mut count = libc::HOST_VM_INFO64_COUNT;

    // `libc` marks `mach_host_self` as deprecated and gives the `mach2` crate
    // as the answer. qex reads one value from it, and a dependency for one
    // value is a poor exchange. The function itself is not deprecated: it is
    // the interface of the kernel, and it does not go away.
    #[allow(deprecated)]
    let rc = unsafe {
        libc::host_statistics64(
            libc::mach_host_self(),
            libc::HOST_VM_INFO64,
            &mut stats as *mut _ as *mut libc::integer_t,
            &mut count,
        )
    };
    if rc != 0 {
        return None;
    }

    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;

    // WHICH PAGES A NEW JOB CAN USE.
    //
    // The free pages are not the answer on macOS. macOS keeps the memory of
    // the machine in use, and it gives the memory back when a program asks for
    // it. A count of the free pages alone thus says that a machine with 16GB
    // has 300MB, and qex would then keep each job in the queue for ever on a
    // machine that has no fault.
    //
    // These four kinds of page go to a new job with no operation to the disk:
    //
    //   free         nothing holds them
    //   inactive     a program had them, and the kernel can take them back
    //   purgeable    a program said that the kernel can discard them
    //   speculative  the kernel read them before a program asked
    //
    // This total is higher than the memory that a job receives in the worst
    // case, and that is the correct direction on macOS. macOS compresses memory
    // and writes it to the disk; it does not stop a program for memory in the
    // way that the Linux out-of-memory killer does. A number that is too low
    // stops each job for ever, which is a fault with no remedy. A number that is
    // a little high makes the machine slow, which the user can see and correct.
    let usable = stats.free_count as u64
        + stats.inactive_count as u64
        + stats.purgeable_count as u64
        + stats.speculative_count as u64;

    Some(usable * page_size)
}

#[cfg(target_os = "macos")]
pub fn process_info(pid: i32) -> Option<ProcessInfo> {
    if pid <= 0 {
        return None;
    }
    let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let rc = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDTBSDINFO,
            0,
            info.as_mut_ptr().cast(),
            size,
        )
    };
    if rc != size {
        return None;
    }
    let info = unsafe { info.assume_init() };
    let name = unsafe { std::ffi::CStr::from_ptr(info.pbi_comm.as_ptr()) }
        .to_string_lossy()
        .into_owned();
    // `NODEV` says that the process has no controlling terminal.
    let terminal = info.e_tdev != u32::MAX;

    let mut paths = std::mem::MaybeUninit::<libc::proc_vnodepathinfo>::uninit();
    let paths_size = std::mem::size_of::<libc::proc_vnodepathinfo>() as libc::c_int;
    let rc = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            paths.as_mut_ptr().cast(),
            paths_size,
        )
    };
    let cwd = if rc == paths_size {
        let paths = unsafe { paths.assume_init() };
        // libc declares the path as a two-dimensional array of bytes.
        let path = unsafe {
            std::ffi::CStr::from_ptr(paths.pvi_cdir.vip_path.as_ptr().cast::<libc::c_char>())
        }
        .to_string_lossy()
        .into_owned();
        (!path.is_empty()).then_some(path)
    } else {
        None
    };

    Some(ProcessInfo {
        ppid: info.pbi_ppid as i32,
        start: Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec),
        name,
        cwd,
        terminal,
    })
}
