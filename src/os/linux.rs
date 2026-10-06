//! The Linux versions of the memory and process-inspection functions of `sys`.

use crate::sys::{GroupUsage, ProcessInfo};

/// Gives the quantity of physical memory in bytes.
#[cfg(target_os = "linux")]
pub fn total_memory() -> u64 {
    meminfo_field("MemTotal:").unwrap_or(0)
}

/// Gives the quantity of memory that a new process can use now.
///
/// The machine can supply this memory without swap.
///
/// On Linux this value is `MemAvailable`. That value includes the page cache
/// that the kernel can reclaim, so it is more accurate than `MemFree`.
/// On macOS this value is the total of the free pages and the inactive pages.
#[cfg(target_os = "linux")]
pub fn available_memory() -> u64 {
    meminfo_field("MemAvailable:").unwrap_or_else(total_memory)
}

/// Gives the memory pressure as a value from 0 to 100.
///
/// The result is `None` if the platform does not supply this measurement.
///
/// On Linux the value is the PSI `some avg10` field of `/proc/pressure/memory`.
/// It is the percentage of the last 10 seconds in which one task or more
/// stopped and waited for memory. This value increases before the quantity of
/// free memory decreases, so it is an earlier warning.
///
/// macOS does not have an equivalent measurement. The result is `None` there,
/// and the caller uses the free memory test only.
#[cfg(target_os = "linux")]
pub fn memory_pressure() -> Option<f64> {
    let text = std::fs::read_to_string("/proc/pressure/memory").ok()?;
    let some = text.lines().find(|l| l.starts_with("some "))?;
    let field = some.split_whitespace().find(|f| f.starts_with("avg10="))?;
    field.trim_start_matches("avg10=").parse().ok()
}

#[cfg(target_os = "linux")]
fn meminfo_field(key: &str) -> Option<u64> {
    let text = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = text.lines().find(|l| l.starts_with(key))?;
    // Each line has this format: "MemTotal:       29316304 kB"
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
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
