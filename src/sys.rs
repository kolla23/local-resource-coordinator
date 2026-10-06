// Modified by the local-resource-coordinator fork, 2026-10-05: the memory and clock bodies moved to src/os/ unchanged; these functions forward to them (R1 of docs/fork/PORT_PLAN.md).
// Modified by the local-resource-coordinator fork, 2026-10-06: R2a of docs/fork/PORT_PLAN.md, the process identity in src/os/unix.rs.
//! This module reads the machine capacity and the current machine load.
//! It also holds the process functions that qex needs.
//!
//! Each function has a Linux version and a macOS version. If a load measurement
//! is not available, the function gives a safe default value. It does not give
//! an error. A measurement that qex cannot read must not stop a job.

use std::time::{SystemTime, UNIX_EPOCH};

/// Gives the number of cores that this machine can use.
pub fn cpu_count() -> u64 {
    std::thread::available_parallelism()
        .map(|n| n.get() as u64)
        .unwrap_or(1)
}

pub fn total_memory() -> u64 {
    crate::os::total_memory()
}

pub fn available_memory() -> u64 {
    crate::os::available_memory()
}

#[cfg(target_os = "linux")]
pub fn memory_pressure() -> Option<f64> {
    crate::os::memory_pressure()
}

#[cfg(not(target_os = "linux"))]
pub fn memory_pressure() -> Option<f64> {
    None
}

pub fn boot_id() -> String {
    crate::os::boot_id()
}

pub fn boot_time_secs() -> Option<u64> {
    crate::os::boot_time_secs()
}

pub fn pid_alive(pid: i32) -> bool {
    crate::os::pid_alive(pid)
}

pub fn job_pid_alive(pid: i32) -> bool {
    crate::os::job_pid_alive(pid)
}

pub fn own_pid_alive(pid: i32) -> bool {
    crate::os::own_pid_alive(pid)
}

pub fn same_process_start(pid: i32, recorded: Option<u64>) -> bool {
    crate::os::same_process_start(pid, recorded)
}

pub fn process_start_token(pid: i32) -> Option<u64> {
    crate::os::process_start_token(pid)
}

pub fn pid_namespace() -> Option<String> {
    crate::os::pid_namespace()
}

/// What qex reads about one process for the chain of a submission.
pub struct ProcessInfo {
    pub ppid: i32,
    pub start: Option<u64>,
    /// The name of the program, as the system gives it.
    pub name: String,
    pub cwd: Option<String>,
    pub terminal: bool,
}

/// Reads the parent, the start time, the name, the directory and the terminal
/// of one process. Gives `None` when the process does not exist or when the
/// system refuses to say.
#[cfg(target_os = "linux")]
pub fn process_info(pid: i32) -> Option<ProcessInfo> {
    if pid <= 0 {
        return None;
    }
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // The command name is in parentheses and can hold spaces and `)`. The
    // stable fields start after the LAST `)`.
    let open = stat.find('(')?;
    let close = stat.rfind(')')?;
    let name = stat.get(open + 1..close)?.to_string();
    let fields: Vec<&str> = stat[close + 1..].split_whitespace().collect();
    // After the name: state, ppid, pgrp, session, tty_nr, ... and the start
    // time is the 20th of them.
    let ppid = fields.get(1)?.parse().ok()?;
    let tty: i64 = fields.get(4)?.parse().ok()?;
    let start = fields.get(19).and_then(|f| f.parse().ok());
    // The directory of a process of another user is refused. That is not a
    // fault: the chain then names the process with no directory.
    let cwd = std::fs::read_link(format!("/proc/{pid}/cwd"))
        .ok()
        .map(|p| p.to_string_lossy().into_owned());
    Some(ProcessInfo {
        ppid,
        start,
        name,
        cwd,
        terminal: tty != 0,
    })
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

/// Gives the process id at the other end of a socket, as THIS machine
/// numbers it.
///
/// The kernel wrote this number when the caller connected, so a caller cannot
/// choose it, and a caller in a container gets the number that the machine of
/// the coordinator gives it. `None` says that the system gave no number, or
/// gave 0: a process that this pid namespace cannot see.
pub fn peer_pid(stream: &std::os::unix::net::UnixStream) -> Option<i32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    #[cfg(target_os = "linux")]
    let pid = {
        let mut cred = std::mem::MaybeUninit::<libc::ucred>::zeroed();
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return None;
        }
        unsafe { cred.assume_init() }.pid
    };
    #[cfg(target_os = "macos")]
    let pid = {
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&mut pid as *mut libc::pid_t).cast(),
                &mut len,
            )
        };
        if rc != 0 {
            return None;
        }
        pid
    };
    (pid > 0).then_some(pid)
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

/// Gives the number of seconds after the Unix epoch.
///
/// qex writes each time value as an integer. A reader can then compare the
/// times in a status file without a date library.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_capacity_is_plausible() {
        assert!(cpu_count() >= 1);
        let total = total_memory();
        assert!(total > 0, "total memory probe returned zero");
        assert!(
            available_memory() <= total,
            "available memory exceeds total"
        );
    }

    #[test]
    fn pressure_is_a_percentage_when_reported() {
        if let Some(p) = memory_pressure() {
            assert!((0.0..=100.0).contains(&p), "pressure {p} out of range");
        }
    }

    #[test]
    fn liveness_check_agrees_about_this_process() {
        assert!(pid_alive(std::process::id() as i32));
        assert!(!pid_alive(-1));
        // For kill(2), the pid 0 means the current process group. qex must not
        // accept 0 as the pid of a live job.
        assert!(!pid_alive(0));
    }

    #[test]
    fn a_process_that_does_not_lead_its_group_is_not_a_job() {
        // The test process runs inside the group of the test runner, so it is
        // alive and it is not a group leader — exactly the shape of a stranger
        // that took the number of a job.
        let me = std::process::id() as i32;
        if unsafe { libc::getpgid(me) } != me {
            assert!(pid_alive(me));
            assert!(!job_pid_alive(me));
        }
        // A group leader is a job candidate. Make one, and let it wait.
        use std::os::unix::process::CommandExt;
        let child = std::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .spawn()
            .expect("starting a group leader");
        let pid = child.id() as i32;
        assert!(job_pid_alive(pid));
        // After the process stops, the number is not a job.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        let mut child = child;
        child.wait().ok();
        assert!(!job_pid_alive(pid));
        assert!(!job_pid_alive(-1));
        assert!(!job_pid_alive(0));
    }

    #[test]
    fn the_start_token_names_one_start_of_a_process() {
        let me = std::process::id() as i32;
        let token = process_start_token(me);
        assert!(token.is_some(), "no start token for the test process");
        // The value is stable across two reads of one process.
        assert_eq!(token, process_start_token(me));
        assert!(process_start_token(-1).is_none());
        assert!(process_start_token(0).is_none());

        // A record without a value loses the test only; a record with a value
        // demands that exact value.
        assert!(same_process_start(me, None));
        assert!(same_process_start(me, token));
        assert!(!same_process_start(me, token.map(|t| t + 1)));
        // A process that shows no start time is not proven to be the recorded
        // one. The pid -1 has none, and the answer for a recorded value must
        // be no, not yes.
        assert!(!same_process_start(-1, Some(42)));
        assert!(same_process_start(-1, None));
    }

    #[test]
    fn a_process_of_another_user_is_never_the_supervisor() {
        let me = std::process::id() as i32;
        assert!(own_pid_alive(me));
        assert!(!own_pid_alive(-1));
        assert!(!own_pid_alive(0));
        // The init process belongs to root. `pid_alive` counts the permission
        // refusal as alive, which is correct for a peer of another user;
        // `own_pid_alive` must not, because the supervisor is always this
        // user. (As root there is no refusal, so the test would test nothing.)
        if unsafe { libc::getuid() } != 0 {
            assert!(pid_alive(1));
            assert!(!own_pid_alive(1));
        }
    }
}

/// The resources that one process group uses now.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GroupUsage {
    /// The memory of every process of the group, in bytes.
    pub rss: u64,
    /// The CPU time of every process of the group, in seconds.
    pub cpu_secs: f64,
    /// The number of processes in the group.
    pub processes: usize,
}

/// Measures the processes of one process group.
///
/// This function compares the process group id, which is a number. It does not
/// read a command line, so it cannot match a command that holds the word `qex`.
/// That fault is the reason for this program.
#[cfg(target_os = "linux")]
pub fn group_usage(pgid: i32) -> GroupUsage {
    let mut out = GroupUsage::default();
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
    let ticks = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as f64;

    let Ok(entries) = std::fs::read_dir("/proc") else {
        return out;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if name.parse::<i32>().is_err() {
            continue;
        }

        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // The command of a process can hold a space or a bracket, and it is
        // inside brackets. Read the fields after the last bracket.
        let Some(rest) = stat.rsplit_once(") ") else {
            continue;
        };
        let fields: Vec<&str> = rest.1.split_whitespace().collect();
        // After the command, field 1 is the state and field 3 is the group.
        if fields.len() < 22 {
            continue;
        }
        let Ok(group) = fields[2].parse::<i32>() else {
            continue;
        };
        if group != pgid {
            continue;
        }

        let utime: f64 = fields[11].parse().unwrap_or(0.0);
        let stime: f64 = fields[12].parse().unwrap_or(0.0);
        let rss_pages: u64 = fields[21].parse().unwrap_or(0);

        out.cpu_secs += (utime + stime) / ticks;
        out.rss += rss_pages * page;
        out.processes += 1;
    }
    out
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

pub fn stamp_text(epoch_secs: u64) -> String {
    crate::os::stamp_text(epoch_secs)
}

pub fn near_stamp_text(epoch_secs: u64, now: u64) -> String {
    crate::os::near_stamp_text(epoch_secs, now)
}

pub fn rfc3339(epoch_secs: u64) -> String {
    crate::os::rfc3339(epoch_secs)
}

pub fn clock_text(epoch_secs: u64) -> String {
    crate::os::clock_text(epoch_secs)
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
