// Modified by the local-resource-coordinator fork, 2026-10-05: the memory and clock bodies moved to src/os/ unchanged; these functions forward to them (R1 of docs/fork/PORT_PLAN.md).
// Modified by the local-resource-coordinator fork, 2026-10-06: R2a of docs/fork/PORT_PLAN.md, the process identity in src/os/unix.rs.
// Modified by the local-resource-coordinator fork, 2026-10-06: R2b of docs/fork/PORT_PLAN.md, the process inspection in src/os/.
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

pub fn process_info(pid: i32) -> Option<ProcessInfo> {
    crate::os::process_info(pid)
}

pub fn submitter_chain() -> Vec<crate::job::Ancestor> {
    crate::os::submitter_chain()
}

pub fn chain_from(pid: i32) -> Vec<crate::job::Ancestor> {
    crate::os::chain_from(pid)
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

pub fn process_exe(pid: i32) -> Option<std::path::PathBuf> {
    crate::os::process_exe(pid)
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

pub fn group_usage(pgid: i32) -> GroupUsage {
    crate::os::group_usage(pgid)
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
