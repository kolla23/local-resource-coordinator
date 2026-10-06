//! The boot time, the local-time text and the process identity of `sys`: one copy for Linux
//! and macOS, with the platform-specific parts inside the functions.

/// Gives the moment when this machine started, in seconds after the Unix epoch.
///
/// A file that was written BEFORE this moment was written in an earlier start
/// of the machine. Recovery uses this to date a record that has no `boot_id`:
/// an old version of qex wrote no identifier, and the process tests alone
/// cannot see a restart.
pub fn boot_time_secs() -> Option<u64> {
    #[cfg(target_os = "linux")]
    if let Ok(text) = std::fs::read_to_string("/proc/stat") {
        if let Some(line) = text.lines().find(|l| l.starts_with("btime ")) {
            return line.split_whitespace().nth(1).and_then(|f| f.parse().ok());
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut tv = libc::timeval {
            tv_sec: 0,
            tv_usec: 0,
        };
        let mut len = std::mem::size_of::<libc::timeval>();
        let rc = unsafe {
            libc::sysctlbyname(
                c"kern.boottime".as_ptr(),
                &mut tv as *mut _ as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc == 0 && tv.tv_sec > 0 {
            return Some(tv.tv_sec as u64);
        }
    }
    None
}

/// Reads a moment in the time zone of the machine.
fn local_parts(epoch_secs: u64) -> libc::tm {
    // The type comes from `localtime_r`. Do not name it: on musl the name
    // `libc::time_t` is deprecated, because that type becomes 64 bits.
    let t = epoch_secs as _;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut parts);
    }
    parts
}

/// Gives the offset of a moment from UTC, as `+01:00`.
fn offset_text(parts: &libc::tm) -> String {
    // The type is `c_long`, which has 32 bits on some systems.
    #[allow(clippy::unnecessary_cast)]
    let offset = parts.tm_gmtoff as i64;
    let sign = if offset < 0 { '-' } else { '+' };
    let minutes = offset.abs() / 60;
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

/// Gives a moment as `2026-09-05 08:10 +01:00`: the date, the minute and the
/// offset from UTC.
///
/// A reader of a pause can be on a different machine, or read the line a day
/// later. A time of day with no date and no offset names a different moment
/// to each such reader.
pub fn stamp_text(epoch_secs: u64) -> String {
    let p = local_parts(epoch_secs);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02} {}",
        p.tm_year + 1900,
        p.tm_mon + 1,
        p.tm_mday,
        p.tm_hour,
        p.tm_min,
        offset_text(&p)
    )
}

/// Gives a moment as `08:10 +01:00` when it is on the same day as `now`, and
/// as `stamp_text` gives it when it is not.
pub fn near_stamp_text(epoch_secs: u64, now: u64) -> String {
    let p = local_parts(epoch_secs);
    let n = local_parts(now);
    if (p.tm_year, p.tm_yday) != (n.tm_year, n.tm_yday) {
        return stamp_text(epoch_secs);
    }
    format!("{:02}:{:02} {}", p.tm_hour, p.tm_min, offset_text(&p))
}

/// Gives a moment in the form of RFC 3339, with the offset of the machine:
/// `2026-09-05T08:10:00+01:00`.
pub fn rfc3339(epoch_secs: u64) -> String {
    let p = local_parts(epoch_secs);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}",
        p.tm_year + 1900,
        p.tm_mon + 1,
        p.tm_mday,
        p.tm_hour,
        p.tm_min,
        p.tm_sec,
        offset_text(&p)
    )
}

/// Gives the time of day as `HH:MM:SS`, in the time zone of the machine.
pub fn clock_text(epoch_secs: u64) -> String {
    // The type comes from `localtime_r`. Do not name it: on musl the name
    // `libc::time_t` is deprecated, because that type becomes 64 bits.
    let t = epoch_secs as _;
    let mut parts: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&t, &mut parts);
    }
    format!(
        "{:02}:{:02}:{:02}",
        parts.tm_hour, parts.tm_min, parts.tm_sec
    )
}

/// Gives an identifier for the current start of the machine.
///
/// qex deletes a peer record that has a different identifier. The system uses
/// each pid again after a restart. Without this test, an old record can look
/// like a live process.
pub fn boot_id() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(id) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
            return id.trim().to_string();
        }
    }
    #[cfg(target_os = "macos")]
    {
        if let Some(boot) = sysctl_boottime() {
            return boot;
        }
    }
    // Without this identifier, qex loses the restart test only. It continues to
    // test each peer process for life.
    "unknown".to_string()
}

#[cfg(target_os = "macos")]
fn sysctl_boottime() -> Option<String> {
    boot_time_secs().map(|secs| format!("boot-{secs}"))
}

/// Tests if a process is alive.
///
/// qex uses this function to delete the records of dead peers. It also uses the
/// function to find a coordinator that stopped and left its files.
///
/// For a live process of a different user, `kill(pid, 0)` gives `EPERM`. That
/// result also shows that the process is alive.
pub fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Tests if a JOB process is alive, with a guard against the reuse of its pid.
///
/// The supervisor makes each job process the leader of its own process group.
/// The machine uses each pid again after the process stops, but a new process
/// with that number is almost never the leader of a group with the same
/// number. A pid that is not a group leader is therefore not the job.
///
/// Unlike `pid_alive`, a refusal of permission does not count as alive here.
/// Each caller of this function sends a signal to the group of the pid when
/// the answer is yes, and qex must not signal a process that it cannot prove
/// is the job.
pub fn job_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::getpgid(pid) == pid }
}

/// Tests if a process of THIS USER is alive.
///
/// Unlike `pid_alive`, a refusal of permission does not count as alive. The
/// supervisor of a job always runs as the user of the coordinator, so a pid
/// that `kill(pid, 0)` refuses belongs to somebody else: the machine gave the
/// number of the supervisor to a new process.
pub fn own_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Tests if the process that holds `pid` NOW is the process that a record
/// named, by its start time.
///
/// `recorded` is the value that `process_start_token` gave when the record was
/// written. `None` comes from a record of an earlier version of qex, which
/// wrote no value; for such a record qex loses this test only and the answer
/// is yes. A recorded value that the current process does not show — because
/// the value differs, or because the current process will not show a start
/// time although the record has one — is a no: qex must not act on a process
/// that it cannot prove is the recorded one.
pub fn same_process_start(pid: i32, recorded: Option<u64>) -> bool {
    match recorded {
        None => true,
        Some(recorded) => process_start_token(pid) == Some(recorded),
    }
}

/// Gives a value that identifies ONE START of a process.
///
/// The machine uses each pid again. Two processes that had one pid started at
/// different times, so a recorded value that differs from the current value
/// shows that the recorded process stopped and a stranger holds the number.
///
/// The unit of the value differs between systems. Compare two values for
/// equality only; never read the value as a time.
pub fn process_start_token(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        // The command name can hold spaces and `)`. The stable fields start
        // after the LAST `)`, and `starttime` is the 20th of them.
        if let Some((_, rest)) = stat.rsplit_once(')') {
            return rest.split_whitespace().nth(19).and_then(|f| f.parse().ok());
        }
    }
    #[cfg(target_os = "macos")]
    {
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
        if rc == size {
            let info = unsafe { info.assume_init() };
            return Some(info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec);
        }
    }
    // Without this value, qex loses the reuse test only. It continues to test
    // the process for life.
    None
}

/// Names the pid namespace of this process.
///
/// A process id has a meaning in one pid namespace only. A record that holds
/// process ids also holds this name, and a reader with a different name must
/// not test those ids: it would find a stranger, or nothing, and report a
/// state that is not true. `None` is a system with one namespace (macOS), or
/// a system that refused to say; two `None` values are the same namespace.
pub fn pid_namespace() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link("/proc/self/ns/pid")
            .ok()
            .map(|p| p.to_string_lossy().into_owned())
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}
