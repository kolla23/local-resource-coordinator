//! The Linux versions of the memory functions of `sys`.

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
