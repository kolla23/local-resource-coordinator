//! The Unix code of `sys` (boot time, local time, process identity and inspection, the terminal)
//! and of the `paths` mode helpers, with the platform parts inside the functions or behind a `cfg`.

use super::process_info;
#[cfg(not(target_os = "linux"))]
use crate::sys::GroupUsage;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

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
    // A zombie keeps its group and start time, but it is the job that ended (#97).
    unsafe { libc::getpgid(pid) == pid && !process_is_zombie(pid) }
}

/// Says yes for a zombie that still leads its process group: its pid, and so the group id, stay
/// reserved until its parent collects it (#97).
pub fn zombie_group_leader(pid: i32) -> bool {
    pid > 0 && process_is_zombie(pid) && unsafe { libc::getpgid(pid) } == pid
}

/// Tests if a process of THIS USER is alive; a zombie is not.
///
/// Unlike `pid_alive`, a refusal of permission does not count as alive. The
/// supervisor of a job always runs as the user of the coordinator, so a pid
/// that `kill(pid, 0)` refuses belongs to somebody else: the machine gave the
/// number of the supervisor to a new process.
pub fn own_pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    unsafe { libc::kill(pid, 0) == 0 && !process_is_zombie(pid) }
}

/// Says yes when the process ended and its parent has not collected it (state Z or X).
/// No evidence (no /proc entry, macOS refuses) gives false, so callers keep today's answer.
pub fn process_is_zombie(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read(format!("/proc/{pid}/stat")) {
        if let Some(rest) = super::stat_after_name(&stat) {
            return matches!(rest.split_whitespace().next(), Some("Z") | Some("X"));
        }
    }
    #[cfg(target_os = "macos")]
    {
        let mut info = std::mem::MaybeUninit::<libc::proc_bsdinfo>::uninit();
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // XNU looks up a zombie for this flavor only when `arg` is nonzero.
        let rc = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                1,
                info.as_mut_ptr().cast(),
                size,
            )
        };
        if rc == size {
            return unsafe { info.assume_init() }.pbi_status == libc::SZOMB;
        }
    }
    false
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
/// The unit differs between systems; on macOS it is wall-clock time, which a clock step moves.
/// Compare for equality, or for order on Linux within one boot; never read it as a time.
pub fn process_start_token(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    if let Ok(stat) = std::fs::read(format!("/proc/{pid}/stat")) {
        // `starttime` is the 20th field after the command name.
        if let Some(rest) = super::stat_after_name(&stat) {
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

/// Gives the real and the effective uid of a process. `None` when the system will not say,
/// which a caller that needs the owner must treat as a stranger.
pub fn process_uids(pid: i32) -> Option<(u32, u32)> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    if let Ok(status) = std::fs::read(format!("/proc/{pid}/status")) {
        // The line holds the real, effective, saved and file system uids.
        let status = status_after_name(&status)?;
        let line = status.lines().find_map(|l| l.strip_prefix("Uid:"))?;
        let mut uids = line.split_whitespace().map(|f| f.parse::<u32>().ok());
        return Some((uids.next()??, uids.next()??));
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
            return Some((info.pbi_ruid, info.pbi_uid));
        }
    }
    None
}

/// Lists the pids in the process group `pgid`. `None` when the process table cannot be
/// read: a caller must not take that for an empty group.
pub fn group_members(pgid: i32) -> Option<Vec<i32>> {
    if pgid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let mut members = Vec::new();
        for entry in std::fs::read_dir("/proc").ok()? {
            let entry = entry.ok()?;
            let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse().ok()) else {
                continue;
            };
            let stat = match std::fs::read(entry.path().join("stat")) {
                Ok(stat) => stat,
                // A process that ended after the listing has no `stat`; it holds nothing.
                Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ESRCH)) => continue,
                Err(_) => return None,
            };
            // The name may hold any bytes. After the last `)`, field 3 is the group.
            let close = stat.iter().rposition(|&b| b == b')')?;
            let rest = std::str::from_utf8(&stat[close + 1..]).ok()?;
            let group = rest.split_whitespace().nth(2)?.parse::<i32>().ok()?;
            if group == pgid {
                members.push(pid);
            }
        }
        Some(members)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let result = std::process::Command::new("ps")
            .args(["-A", "-o", "pid=,pgid="])
            .output()
            .ok()
            .filter(|r| r.status.success())?;
        let members = String::from_utf8_lossy(&result.stdout)
            .lines()
            .filter_map(|line| {
                let mut fields = line.split_whitespace().map(|f| f.parse::<i32>().ok());
                match (fields.next()?, fields.next()?) {
                    (Some(pid), Some(group)) if group == pgid => Some(pid),
                    _ => None,
                }
            })
            .collect();
        Some(members)
    }
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

/// The pid namespace's name and its pid 1's start, which a later namespace reusing the name
/// lacks; `None` on macOS, when /proc or the boot clock is not this namespace's, or a read fails.
pub fn pid_namespace_identity() -> Option<(String, u64)> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read("/proc/self/status").ok()?;
        let time_ns =
            std::fs::read_link("/proc/self/ns/time").map(|p| p.to_string_lossy().into_owned());
        identity_from(&status, &time_ns, || {
            Some((pid_namespace()?, process_start_token(1)?))
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// `read`'s identity, given only when `status` comes from this namespace's own /proc and the
/// time namespace `time_ns` adds no offset to the start tokens that /proc gives.
#[cfg(any(target_os = "linux", test))]
fn identity_from(
    status: &[u8],
    time_ns: &std::io::Result<String>,
    read: impl FnOnce() -> Option<(String, u64)>,
) -> Option<(String, u64)> {
    if !status_after_name(status).is_some_and(nspid_is_own) || !boot_clock_unshifted(time_ns) {
        return None;
    }
    read()
}

/// The initial time namespace (the kernel's fixed PROC_TIME_INIT_INO) has no boot clock offset,
/// and nor does a kernel without time namespaces, where the link is missing (ENOENT only).
#[cfg(any(target_os = "linux", test))]
fn boot_clock_unshifted(time_ns: &std::io::Result<String>) -> bool {
    match time_ns {
        Ok(name) => name == "time:[4026531834]",
        Err(e) => e.raw_os_error() == Some(libc::ENOENT),
    }
}

/// Says whether a `/proc/self/status` text comes from this namespace's own /proc: its `NSpid:`
/// line has one entry more for each namespace between that /proc's and this one.
#[cfg(any(target_os = "linux", test))]
fn nspid_is_own(status: &str) -> bool {
    status
        .lines()
        .find_map(|l| l.strip_prefix("NSpid:"))
        .is_some_and(|pids| pids.split_whitespace().count() == 1)
}

/// A `/proc/<pid>/status` text after its first line, the `Name:` line, whose value can hold any
/// bytes; `None` when the rest is not UTF-8, as for the whole text before (#131).
#[cfg(any(target_os = "linux", test))]
fn status_after_name(status: &[u8]) -> Option<&str> {
    let rest = status.strip_prefix(b"Name:").map_or(status, |name| {
        name.splitn(2, |&b| b == b'\n').nth(1).unwrap_or_default()
    });
    std::str::from_utf8(rest).ok()
}

/// Gives the chain of processes above this process, from its parent up to
/// the first process of the machine.
///
/// `qex submit` records this chain on the job, and `qex abort` reads it. The
/// walk records EVERY process, and the `context` module decides where the
/// session ends when it compares two chains, so a change to that rule reads
/// the records that exist.
pub fn submitter_chain() -> Vec<crate::job::Ancestor> {
    chain_from(unsafe { libc::getppid() })
}

/// Gives the chain of processes from `pid` upward, `pid` included.
///
/// The coordinator uses this walk for the process at the other end of a
/// socket, so the numbers are the numbers of the machine of the coordinator,
/// whatever pid namespace the caller lives in.
pub fn chain_from(mut pid: i32) -> Vec<crate::job::Ancestor> {
    let mut out = Vec::new();
    // A limit, so a strange process table cannot make an endless loop.
    for _ in 0..64 {
        if pid <= 0 {
            break;
        }
        let Some(info) = process_info(pid) else {
            break;
        };
        out.push(crate::job::Ancestor {
            pid,
            ppid: info.ppid,
            start: info.start,
            // The name of a process is text that the process chose. See
            // `job::safe_name`.
            name: crate::job::safe_name(&info.name),
            cwd: info.cwd,
            terminal: info.terminal,
        });
        pid = info.ppid;
    }
    out
}

/// Gives the program file of a process, when the system says.
pub fn process_exe(pid: i32) -> Option<std::path::PathBuf> {
    if pid <= 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok()
    }
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        let rc = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if rc <= 0 {
            return None;
        }
        buf.truncate(rc as usize);
        Some(std::path::PathBuf::from(
            String::from_utf8_lossy(&buf).into_owned(),
        ))
    }
}

/// Measures the processes of one process group.
///
/// macOS has no `/proc`, so this version reads the output of `ps`.
#[cfg(not(target_os = "linux"))]
pub fn group_usage(pgid: i32) -> GroupUsage {
    let mut out = GroupUsage::default();
    let Ok(result) = std::process::Command::new("ps")
        .args(["-A", "-o", "pgid=,rss=,time="])
        .output()
    else {
        return out;
    };

    for line in String::from_utf8_lossy(&result.stdout).lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }
        if fields[0].parse::<i32>() != Ok(pgid) {
            continue;
        }
        // `ps` gives the memory in kilobytes.
        out.rss += fields[1].parse::<u64>().unwrap_or(0) * 1024;
        out.cpu_secs += parse_ps_time(fields[2]);
        out.processes += 1;
    }
    out
}

/// Reads a time from `ps`, in the form `MM:SS.ss` or `HH:MM:SS`.
#[cfg(not(target_os = "linux"))]
fn parse_ps_time(text: &str) -> f64 {
    let parts: Vec<&str> = text.split(':').collect();
    let mut seconds = 0.0;
    for part in &parts {
        seconds = seconds * 60.0 + part.parse::<f64>().unwrap_or(0.0);
    }
    seconds
}

/// Makes a directory and its parent directories, then sets the mode.
///
/// This function sets the mode after it makes the directory. A permissive umask
/// thus cannot make the directory more open than the `mode` parameter.
pub fn ensure_dir(path: &Path, mode: u32) -> Result<()> {
    ensure_dir_as(
        path,
        mode,
        unsafe { libc::geteuid() },
        &Namespace::detect(),
        &lstat_entry,
    )
}

/// Makes one level at a time and checks each level before it goes below it,
/// so a link that another user puts at a missing level is never followed.
fn ensure_dir_as(path: &Path, mode: u32, euid: u32, ns: &Namespace, look: Look) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut walk = Walk::new(path, euid, ns, look)?;
    // Each missing parent, such as the state directory on a first start, gets
    // the mode too; a parent that exists keeps its own.
    walk.create = Some(mode);
    walk.mode_rule = false;
    walk.run()?;
    // Set the mode here. The umask narrows the mode of a new directory, and a
    // directory that existed keeps its old mode.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .with_context(|| format!("setting mode {mode:o} on {}", path.display()))?;
    Ok(())
}

/// Checks the state directory and every directory and link on the way to it, then
/// makes it private; false when it is missing, since `ensure_dir` checks each level it makes.
pub fn check_state_dir(path: &Path, euid: u32) -> Result<bool> {
    check_state_dir_as(path, euid, &Namespace::detect(), &lstat_entry)
}

fn check_state_dir_as(path: &Path, euid: u32, ns: &Namespace, look: Look) -> Result<bool> {
    // Only what the walk saw is opened: an entry made after its look is checked next time.
    if Walk::new(path, euid, ns, look)?.run()?.is_none() {
        return Ok(false);
    }
    narrow_owned_dir(path, euid)
}

#[cfg(test)]
fn check_owned_dir_as(path: &Path, euid: u32, ns: &Namespace, look: Look) -> Result<()> {
    Walk::new(path, euid, ns, look)?.run().map(|_| ())
}

/// Checks the owner on the opened handle, so the answer is about the directory
/// that is used; gives false when the directory is missing.
fn narrow_owned_dir(path: &Path, euid: u32) -> Result<bool> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let dir = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY)
        .open(path)
    {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e).with_context(|| format!("opening {}", path.display())),
    };
    let meta = dir
        .metadata()
        .with_context(|| format!("reading {}", path.display()))?;
    if meta.uid() != euid {
        let ns = Namespace { overflow: None };
        return Err(refusal(path, path, meta.uid(), euid, &ns));
    }
    if meta.mode() & 0o022 != 0 {
        dir.set_permissions(std::fs::Permissions::from_mode(0o700))
            .with_context(|| {
                format!(
                    "setting mode 700 on {}, which has the mode {:o}",
                    path.display(),
                    meta.mode() & 0o7777
                )
            })?;
    }
    Ok(true)
}

/// What one `lstat` reports about a path.
#[derive(Clone, Copy, Debug)]
struct Entry {
    uid: u32,
    mode: u32,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Dir,
    Link,
    Other,
}

type Look<'a> = &'a dyn Fn(&Path) -> std::io::Result<Entry>;

fn lstat_entry(path: &Path) -> std::io::Result<Entry> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(path)?;
    let kind = if meta.file_type().is_symlink() {
        Kind::Link
    } else if meta.is_dir() {
        Kind::Dir
    } else {
        Kind::Other
    };
    Ok(Entry {
        uid: meta.uid(),
        mode: meta.mode(),
        kind,
    })
}

/// The user namespace qex runs in: `overflow` is the uid that every unmapped
/// user shows as, and None in the initial namespace.
#[derive(Clone, Copy, Debug)]
struct Namespace {
    overflow: Option<u32>,
}

impl Namespace {
    fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            let initial = std::fs::read_to_string("/proc/self/uid_map")
                .map(|map| map.split_whitespace().eq(["0", "0", "4294967295"]))
                .unwrap_or(true);
            if !initial {
                let overflow = std::fs::read_to_string("/proc/sys/kernel/overflowuid")
                    .ok()
                    .and_then(|text| text.trim().parse().ok())
                    .unwrap_or(65534);
                return Self {
                    overflow: Some(overflow),
                };
            }
        }
        Self { overflow: None }
    }
}

/// The most links one path may pass through, like the kernel's limit.
const LINK_LIMIT: usize = 40;

/// In a user namespace root's directories show the overflow uid, as any unmapped user's
/// do; it is trusted above the state directory here only (`/var` to reach `/var/tmp`).
const OVERFLOW_PATHS: [&str; 4] = ["/", "/tmp", "/var", "/var/tmp"];

const SANDBOX_PAGE: &str =
    "https://github.com/kolla23/local-resource-coordinator/blob/main/docs/sandbox.md";

/// One check of a path, a component at a time, following only owned links.
struct Walk<'a> {
    target: PathBuf,
    euid: u32,
    ns: &'a Namespace,
    look: Look<'a>,
    links: usize,
    /// Directories above the last component must not be writable by every user.
    mode_rule: bool,
    /// Make each missing level with this mode, as `ensure_dir` does.
    create: Option<u32>,
}

impl<'a> Walk<'a> {
    fn new(path: &Path, euid: u32, ns: &'a Namespace, look: Look<'a>) -> Result<Self> {
        if ns.overflow == Some(euid) {
            anyhow::bail!(
                "qex runs as the user {euid}, which this user namespace does not map, so it \
                 cannot own a state directory. Start the sandbox with a mapped user \
                 (`unshare -Ur`, or the default mapping of bubblewrap): see {SANDBOX_PAGE}"
            );
        }
        let target = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .context("reading the current directory")?
                .join(path)
        };
        Ok(Self {
            target,
            euid,
            ns,
            look,
            links: 0,
            mode_rule: true,
            create: None,
        })
    }

    fn run(&mut self) -> Result<Option<PathBuf>> {
        let target = self.target.clone();
        self.resolve(&target, true, self.mode_rule)
    }

    /// Gives the real directory that `path` names, or None when a level is missing;
    /// `last` says that its final component is the checked directory itself.
    fn resolve(&mut self, path: &Path, last: bool, mode_rule: bool) -> Result<Option<PathBuf>> {
        use std::path::Component;
        let mut at = PathBuf::from("/");
        let root = (self.look)(&at).context("reading /")?;
        self.check_dir(&at, root, false, mode_rule)?;
        let parts: Vec<Component> = path.components().collect();
        let mut final_seen = false;
        for (i, part) in parts.iter().enumerate() {
            let name = match part {
                Component::Normal(name) => name,
                Component::ParentDir => {
                    at.pop();
                    continue;
                }
                _ => continue,
            };
            let final_part = last && i + 1 == parts.len();
            let next = at.join(name);
            let entry = match (self.look)(&next) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    let Some(mode) = self.create else {
                        return Ok(None);
                    };
                    make_one_level(&next, mode)?;
                    #[cfg(debug_assertions)]
                    pause_after_mkdir(&next);
                    (self.look)(&next).with_context(|| format!("reading {}", next.display()))?
                }
                found => found.with_context(|| format!("reading {}", next.display()))?,
            };
            match entry.kind {
                Kind::Link => {
                    // The user's or root's only: the overflow uid is trusted for directories.
                    if entry.uid != self.euid && entry.uid != 0 {
                        return Err(refusal(&self.target, &next, entry.uid, self.euid, self.ns));
                    }
                    self.links += 1;
                    if self.links > LINK_LIMIT {
                        anyhow::bail!(
                            "qex will not use {}: the path passes through more than \
                             {LINK_LIMIT} links, at {}",
                            self.target.display(),
                            next.display()
                        );
                    }
                    let to = std::fs::read_link(&next)
                        .with_context(|| format!("reading the link {}", next.display()))?;
                    match self.resolve(&at.join(to), final_part, true)? {
                        Some(real) => at = real,
                        None => return Ok(None),
                    }
                }
                Kind::Dir => {
                    self.check_dir(&next, entry, final_part, mode_rule)?;
                    at = next;
                }
                Kind::Other => anyhow::bail!(
                    "qex will not use {}: {} is not a directory",
                    self.target.display(),
                    next.display()
                ),
            }
            final_seen |= final_part;
        }
        // A path that ends at `/` or in `..` names a directory no step checked as the last.
        if last && !final_seen {
            let entry = (self.look)(&at).with_context(|| format!("reading {}", at.display()))?;
            self.check_dir(&at, entry, true, mode_rule)?;
        }
        Ok(Some(at))
    }

    fn check_dir(&self, path: &Path, entry: Entry, last: bool, mode_rule: bool) -> Result<()> {
        self.check_owner(path, entry.uid, last)?;
        if !last && mode_rule && entry.mode & 0o002 != 0 && entry.mode & 0o1000 == 0 {
            anyhow::bail!(
                "qex will not use {}: every user can write to {}, and it has no sticky bit. \
                 Set XDG_STATE_HOME to a directory that only you can change",
                self.target.display(),
                path.display()
            );
        }
        Ok(())
    }

    /// The checked directory must be the user's own; above it, root's will do,
    /// and inside a user namespace the overflow uid at `OVERFLOW_PATHS`.
    fn check_owner(&self, path: &Path, uid: u32, last: bool) -> Result<()> {
        let overflow_path =
            self.ns.overflow == Some(uid) && OVERFLOW_PATHS.iter().any(|p| path == Path::new(p));
        if uid == self.euid || (!last && (uid == 0 || overflow_path)) {
            return Ok(());
        }
        Err(refusal(&self.target, path, uid, self.euid, self.ns))
    }
}

/// A level that another process made first is checked like any other.
fn make_one_level(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    match std::fs::DirBuilder::new().mode(mode).create(path) {
        Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists => {
            Err(e).with_context(|| format!("creating directory {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Pauses qex once after a mkdir in one parent: `<ms> <parent>`. **This variable
/// exists for the tests of qex**, and a release build does not read it.
#[cfg(debug_assertions)]
const PAUSE_AFTER_MKDIR_IN: &str = "QEX_TEST_PAUSE_AFTER_MKDIR_IN";

/// A test of qex pauses here, between a new directory and its read-back, so a
/// sweep can delete the directory (issue #132).
#[cfg(debug_assertions)]
fn pause_after_mkdir(made: &Path) {
    static PAUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let Ok(value) = std::env::var(PAUSE_AFTER_MKDIR_IN) else {
        return;
    };
    let Some((ms, parent)) = value.split_once(' ') else {
        return;
    };
    let Ok(ms) = ms.parse::<u64>() else {
        return;
    };
    // The walk resolved every link in `made`, so the parent must match resolved:
    // on macOS, TMPDIR lies under /var, a link to /private/var (issue #145).
    let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| PathBuf::from(parent));
    if made.parent() != Some(parent.as_path())
        || PAUSED.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }
    eprintln!(
        "qex pauses {ms} ms after it made {} because {PAUSE_AFTER_MKDIR_IN} is set",
        made.display()
    );
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

/// A plain error with no `io::Error` inside, so the sandbox remedy is added
/// only when the owner is the overflow uid.
fn refusal(target: &Path, path: &Path, uid: u32, euid: u32, ns: &Namespace) -> anyhow::Error {
    let sandbox = if ns.overflow == Some(uid) {
        format!(
            " Inside a sandbox, set XDG_STATE_HOME to a directory under /tmp: see {SANDBOX_PAGE}"
        )
    } else {
        String::new()
    };
    anyhow::anyhow!(
        "qex will not use {}: {} belongs to the user {uid}, and qex runs as the user {euid}. \
         Set XDG_STATE_HOME to a directory that you own, or run qex as that user.{sandbox}",
        target.display(),
        path.display()
    )
}

/// Sets 0600 on an open file, also one that existed with a wider mode. Best effort:
/// each such file lies in a 0700 directory, and a refused change must not stop qex.
pub fn make_private(file: &std::fs::File) {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    // Only a regular file with one link that this user owns: a link planted in
    // someone else's state folder must not move the change to a file outside it.
    let Ok(meta) = file.metadata() else { return };
    if !meta.is_file() || meta.nlink() != 1 || meta.uid() != unsafe { libc::geteuid() } {
        return;
    }
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .ok();
}

/// Tests if the standard input is a terminal.
///
/// A command that reads a key needs a terminal. In a pipe or a script there is
/// no key to read.
pub fn stdin_is_terminal() -> bool {
    unsafe { libc::isatty(libc::STDIN_FILENO) == 1 }
}

/// The size of the terminal that this command writes to, as `(rows, columns)`.
///
/// `None` when the output is not a terminal, or when the system does not
/// give a size. A page that has no size writes every job and draws no frame.
pub fn terminal_size() -> Option<(usize, usize)> {
    let mut size: libc::winsize = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) };
    if ok == 0 && size.ws_row > 0 {
        let cols = if size.ws_col > 0 {
            size.ws_col as usize
        } else {
            80
        };
        Some((size.ws_row as usize, cols))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{symlink, PermissionsExt};

    // A unit test: making a pid namespace that sees its parent's /proc needs a user namespace,
    // which a test machine may refuse, and no qex command shows the recorded value's source.
    #[test]
    fn only_a_single_nspid_entry_is_this_namespaces_own_proc() {
        let status = |nspid: &str| format!("Name:\tqex\nPid:\t42\nNSpid:{nspid}\nPPid:\t1\n");
        assert!(nspid_is_own(&status("\t42")));
        assert!(
            !nspid_is_own(&status("\t4242\t42")),
            "two entries: the parent's /proc"
        );
        assert!(!nspid_is_own(&status("")), "no entry");
        assert!(
            !nspid_is_own("Name:\tqex\nPid:\t42\n"),
            "no NSpid line (an old kernel)"
        );

        // `pid_namespace_identity` gives what `identity_from` gives for its /proc text.
        let read = || Some(("pid:[4026532263]".to_string(), 161));
        let initial = Ok("time:[4026531834]".to_string());
        assert_eq!(
            identity_from(status("\t42").as_bytes(), &initial, read),
            read()
        );
        assert_eq!(
            identity_from(status("\t4242\t42").as_bytes(), &initial, read),
            None,
            "the parent's /proc"
        );
        assert_eq!(
            identity_from(b"Name:\tqex\n", &initial, read),
            None,
            "no NSpid line"
        );
        assert_eq!(
            identity_from(b"Name:\tq\xffx\nNSpid:\t42\xff\n", &initial, read),
            None,
            "an NSpid line that is not UTF-8"
        );
        assert_eq!(
            identity_from(
                b"Name:\tq\xffx\nUid:\t1000\xff\nNSpid:\t42\n",
                &initial,
                read
            ),
            None,
            "a Uid line that is not UTF-8"
        );
    }

    // A unit test: a time namespace with an offset needs a user namespace, which a test machine
    // may refuse, and no qex command shows the recorded value's source.
    #[test]
    fn only_a_boot_clock_with_no_offset_gives_a_namespace_identity() {
        let own: &[u8] = b"Name:\tqex\nNSpid:\t42\n";
        let read = || Some(("pid:[4026532263]".to_string(), 161));
        let err = |code| Err(std::io::Error::from_raw_os_error(code));
        let initial = Ok("time:[4026531834]".to_string());
        assert_eq!(
            identity_from(own, &initial, read),
            read(),
            "the initial time namespace"
        );
        let other = Ok("time:[4026532270]".to_string());
        assert_eq!(
            identity_from(own, &other, read),
            None,
            "a time namespace with an offset"
        );
        assert_eq!(
            identity_from(own, &err(libc::ENOENT), read),
            read(),
            "no time namespaces"
        );
        assert_eq!(
            identity_from(own, &err(libc::EACCES), read),
            None,
            "an unreadable link"
        );
    }

    const NO_NS: Namespace = Namespace { overflow: None };

    fn me() -> u32 {
        unsafe { libc::geteuid() }
    }

    /// A uid that is neither the test's user nor root.
    fn other() -> u32 {
        me() + 1
    }

    /// A new path right under /tmp, in its real form (macOS reaches /tmp through a link).
    fn in_tmp(name: &str) -> PathBuf {
        std::fs::canonicalize(crate::testutil::temp_dir())
            .unwrap()
            .join(format!(
                "qex-own-{name}-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ))
    }

    fn fresh(name: &str) -> PathBuf {
        let dir = in_tmp(name);
        std::fs::create_dir(&dir).unwrap();
        dir
    }

    fn mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    /// Root owns what it makes, and root's directories pass; give them to a stranger.
    fn make_foreign(path: &Path) {
        if me() == 0 {
            std::os::unix::fs::lchown(path, Some(4242), Some(4242)).unwrap();
        }
    }

    fn check(path: &Path, euid: u32) -> Result<()> {
        check_owned_dir_as(path, euid, &NO_NS, &lstat_entry)
    }

    fn error(result: Result<()>) -> String {
        format!("{:#}", result.expect_err("the check must refuse"))
    }

    /// `lstat_entry`, with the owner of the named paths replaced by `uid`.
    fn owned_by(paths: Vec<PathBuf>, uid: u32) -> impl Fn(&Path) -> std::io::Result<Entry> {
        move |path| {
            let mut entry = lstat_entry(path)?;
            if paths.iter().any(|p| p == path) {
                entry.uid = uid;
            }
            Ok(entry)
        }
    }

    #[test]
    fn a_state_directory_is_checked_against_its_owner() {
        let t = fresh("owner");
        let state = t.join("qex");
        // Missing: it passes for its parent's owner, and ensure_dir checks it.
        check(&state, me()).unwrap();
        make_foreign(&t);
        let euid = if me() == 0 { 0 } else { other() };
        assert!(error(check(&state, euid)).contains(&format!("{} belongs", t.display())));
        std::fs::remove_dir_all(&t).ok();

        let t = fresh("owner2");
        let state = t.join("qex");
        std::fs::create_dir(&state).unwrap();
        check(&state, me()).unwrap();
        let text = error(check(&state, other()));
        assert!(
            text.contains(&format!("belongs to the user {}", me())),
            "{text}"
        );
        assert!(text.contains("XDG_STATE_HOME"), "{text}");
        // Root's directories pass above the state directory, never as it.
        if me() != 0 {
            assert!(error(check(Path::new("/"), me())).contains(": / belongs to the user 0"));
        }

        // Behind the user's own links, at the state directory and above it.
        symlink(&state, t.join("link")).unwrap();
        check(&t.join("link"), me()).unwrap();
        symlink(&t, t.join("up")).unwrap();
        check(&t.join("up/qex"), me()).unwrap();
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_directory_above_the_state_directory_must_not_be_open_to_every_user() {
        let t = fresh("open");
        let open = t.join("open");
        std::fs::create_dir_all(open.join("qex")).unwrap();
        mode(&open, 0o777);
        assert!(error(check(&open.join("qex"), me())).contains("no sticky bit"));
        mode(&open, 0o1777);
        check(&open.join("qex"), me()).unwrap();
        mode(&open, 0o775);
        check(&open.join("qex"), me()).unwrap();
        // The state directory's own mode is narrowed by its caller, not refused here.
        mode(&open, 0o755);
        mode(&open.join("qex"), 0o777);
        check(&open.join("qex"), me()).unwrap();
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn every_directory_on_the_way_to_a_link_target_is_checked() {
        let t = fresh("hop");
        std::fs::create_dir_all(t.join("final")).unwrap();
        std::fs::create_dir(t.join("open")).unwrap();
        symlink(t.join("final"), t.join("open/hop")).unwrap();
        symlink(t.join("open/hop"), t.join("qex")).unwrap();
        mode(&t.join("open"), 0o777);
        let text = error(check(&t.join("qex"), me()));
        assert!(
            text.contains(&format!("to {},", t.join("open").display())),
            "{text}"
        );
        if me() == 0 {
            mode(&t.join("open"), 0o755);
            make_foreign(&t.join("open"));
            let text = error(check(&t.join("qex"), me()));
            assert!(text.contains("open belongs to the user 4242"), "{text}");
        }
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn a_relative_state_directory_is_checked_from_the_current_directory() {
        let seen = std::cell::RefCell::new(Vec::new());
        let look = |path: &Path| {
            seen.borrow_mut().push(path.to_path_buf());
            lstat_entry(path)
        };
        let _ = check_owned_dir_as(Path::new("no-such-dir/qex"), me(), &NO_NS, &look);
        let cwd = std::env::current_dir().unwrap();
        let first = cwd.components().nth(1).unwrap();
        let seen = seen.into_inner();
        assert!(seen.contains(&Path::new("/").join(first)), "{seen:?}");
        assert!(!seen.contains(&PathBuf::from("/no-such-dir")), "{seen:?}");
    }

    #[test]
    fn more_than_forty_links_are_refused() {
        let t = fresh("links");
        std::fs::create_dir(t.join("real")).unwrap();
        symlink("real", t.join("l40")).unwrap();
        for i in (0..40).rev() {
            symlink(format!("l{}", i + 1), t.join(format!("l{i}"))).unwrap();
        }
        check(&t.join("l1"), me()).unwrap();
        assert!(error(check(&t.join("l0"), me())).contains("more than 40 links"));
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn the_overflow_uid_is_trusted_only_at_fixed_paths_in_a_user_namespace() {
        let t = fresh("overflow");
        let sticky = t.join("sticky");
        std::fs::create_dir_all(sticky.join("qex")).unwrap();
        mode(&sticky, 0o1777);
        let x = other();
        let ns = Namespace { overflow: Some(x) };
        let look = owned_by(vec![sticky.clone()], x);
        let text = error(check_owned_dir_as(&sticky.join("qex"), me(), &ns, &look));
        assert!(
            text.contains(&format!("sticky belongs to the user {x}")),
            "{text}"
        );
        assert!(text.contains("docs/sandbox.md"), "{text}");
        let text = error(check_owned_dir_as(&sticky.join("qex"), me(), &NO_NS, &look));
        assert!(text.contains("sticky belongs"), "{text}");
        assert!(!text.contains("docs/sandbox.md"), "{text}");

        // Run as the overflow uid, qex can own nothing: refused before any path.
        let text = error(check_owned_dir_as(
            &sticky.join("qex"),
            x,
            &ns,
            &lstat_entry,
        ));
        assert!(text.contains("does not map"), "{text}");
        let text = error(check_owned_dir_as(
            &sticky.join("qex"),
            x,
            &NO_NS,
            &lstat_entry,
        ));
        assert!(text.contains(&format!("{}", t.display())), "{text}");
        let text = error(ensure_dir_as(
            &t.join("new/run"),
            0o700,
            x,
            &ns,
            &lstat_entry,
        ));
        assert!(text.contains("does not map"), "{text}");
        assert!(!t.join("new").exists());
        std::fs::remove_dir_all(&t).ok();
    }

    // Linux only: macOS has no user namespaces, and its /tmp is a link, which a
    // faked owner would turn into a foreign link.
    #[cfg(target_os = "linux")]
    #[test]
    fn root_directories_that_show_the_overflow_uid_pass_in_a_user_namespace() {
        let t = fresh("overflow-root");
        let x = other();
        let root_dirs = ["/", "/tmp", "/var", "/var/tmp"];
        let look = owned_by(root_dirs.iter().map(PathBuf::from).collect(), x);
        let ns = Namespace { overflow: Some(x) };
        check_owned_dir_as(&t.join("qex"), me(), &ns, &look).unwrap();
        let var_tmp = Path::new("/var/tmp/no-such-qex-dir/qex");
        check_owned_dir_as(var_tmp, me(), &ns, &look).unwrap();
        let text = error(check_owned_dir_as(&t.join("qex"), me(), &NO_NS, &look));
        assert!(
            text.contains(&format!(": / belongs to the user {x}")),
            "{text}"
        );
        std::fs::remove_dir_all(&t).ok();
    }

    // Linux only, as above: the walk reaches the real /var/tmp.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_link_that_shows_the_overflow_uid_is_refused_at_a_fixed_path() {
        let x = other();
        let look = |path: &Path| {
            let mut entry = lstat_entry(path)?;
            if ["/", "/var", "/var/tmp"]
                .iter()
                .any(|p| path == Path::new(p))
            {
                entry.uid = x;
            }
            if path == Path::new("/var/tmp") {
                entry.kind = Kind::Link;
            }
            Ok(entry)
        };
        let ns = Namespace { overflow: Some(x) };
        let path = Path::new("/var/tmp/no-such-qex-dir/qex");
        let text = error(check_owned_dir_as(path, me(), &ns, &look));
        assert!(
            text.contains(&format!("/var/tmp belongs to the user {x}")),
            "{text}"
        );
    }

    #[test]
    fn a_state_directory_that_appears_after_the_walk_is_not_used() {
        let t = fresh("appears");
        std::fs::create_dir(t.join("target")).unwrap();
        mode(&t.join("target"), 0o777);
        symlink(t.join("target"), t.join("qex")).unwrap();
        // The walk saw no `qex`: it appeared after the look, made by anyone.
        let look = |path: &Path| {
            if path == t.join("qex") {
                return Err(std::io::ErrorKind::NotFound.into());
            }
            lstat_entry(path)
        };
        let found = check_state_dir_as(&t.join("qex"), me(), &NO_NS, &look).unwrap();
        assert!(!found, "a directory the walk did not see is not used");
        let left = std::fs::metadata(t.join("target"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(left & 0o777, 0o777, "nothing was done through the link");
        std::fs::remove_dir_all(&t).ok();
    }

    #[test]
    fn ensure_dir_refuses_a_level_another_user_owns() {
        let t = fresh("ensure");
        mode(&t, 0o755);
        let text = error(ensure_dir_as(&t, 0o700, other(), &NO_NS, &lstat_entry));
        assert!(text.contains("belongs to the user"), "{text}");
        let left = std::fs::metadata(&t).unwrap().permissions().mode();
        assert_eq!(left & 0o7777, 0o755, "a refused directory keeps its mode");

        // A missing level that another user made first: a directory, then a link.
        make_foreign(&t);
        let text = error(ensure_dir_as(
            &t.join("b/c"),
            0o700,
            other(),
            &NO_NS,
            &lstat_entry,
        ));
        assert!(text.contains(&format!("{} belongs", t.display())), "{text}");
        assert!(!t.join("b").exists());
        let elsewhere = fresh("ensure-target");
        let link = in_tmp("ensure-link");
        symlink(&elsewhere, &link).unwrap();
        make_foreign(&link);
        let text = error(ensure_dir_as(
            &link.join("b/c"),
            0o700,
            other(),
            &NO_NS,
            &lstat_entry,
        ));
        assert!(
            text.contains(&format!("{} belongs", link.display())),
            "{text}"
        );
        assert!(!elsewhere.join("b").exists());
        for path in [&t, &elsewhere, &link] {
            std::fs::remove_dir_all(path)
                .or_else(|_| std::fs::remove_file(path))
                .ok();
        }
    }

    #[test]
    fn a_level_made_by_another_process_first_is_checked() {
        let made = fresh("eexist");
        // The first look misses the level, as when it appears just before the mkdir.
        let missed = std::cell::Cell::new(false);
        let look = |path: &Path| {
            if path == made && !missed.replace(true) {
                return Err(std::io::ErrorKind::NotFound.into());
            }
            lstat_entry(path)
        };
        ensure_dir_as(&made, 0o700, me(), &NO_NS, &look).unwrap();
        assert!(missed.get(), "the level was missed once");

        let link = in_tmp("eexist-link");
        symlink(&made, &link).unwrap();
        make_foreign(&link);
        let missed = std::cell::Cell::new(false);
        let look = |path: &Path| {
            if path == link && !missed.replace(true) {
                return Err(std::io::ErrorKind::NotFound.into());
            }
            owned_by(vec![made.clone()], other())(path)
        };
        let text = error(ensure_dir_as(&link, 0o700, other(), &NO_NS, &look));
        assert!(
            text.contains(&format!("{} belongs", link.display())),
            "{text}"
        );
        std::fs::remove_file(&link).ok();
        std::fs::remove_dir_all(&made).ok();
    }
}
