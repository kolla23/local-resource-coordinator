//! The boot time and the local-time text of `sys`, the same on every Unix.

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
