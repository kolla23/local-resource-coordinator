//! The platform code behind `sys`: one file per platform, and `unix` for the code that has
//! one copy for Linux and macOS. `sys` keeps the signatures and forwards here.

mod unix;
pub use unix::{
    boot_id, boot_time_secs, clock_text, job_pid_alive, near_stamp_text, own_pid_alive, pid_alive,
    pid_namespace, process_start_token, rfc3339, same_process_start, stamp_text,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{available_memory, memory_pressure, total_memory};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{available_memory, total_memory};
