//! The platform code behind `sys`: one file per platform, and `unix` for the Unix code that
//! is shared or behind its own `cfg`. `sys` keeps the signatures and forwards here.

mod unix;
#[cfg(not(target_os = "linux"))]
pub use unix::group_usage;
pub use unix::{
    boot_id, boot_time_secs, chain_from, clock_text, job_pid_alive, near_stamp_text, own_pid_alive,
    pid_alive, pid_namespace, process_exe, process_start_token, rfc3339, same_process_start,
    stamp_text, submitter_chain,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{available_memory, group_usage, memory_pressure, process_info, total_memory};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{available_memory, process_info, total_memory};
