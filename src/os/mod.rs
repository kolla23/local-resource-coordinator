//! The platform code behind `sys`: one file per platform, and `unix` for the code that is
//! the same on every Unix. `sys` keeps the signatures and forwards here.

mod unix;
pub use unix::{boot_time_secs, clock_text, near_stamp_text, rfc3339, stamp_text};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{available_memory, memory_pressure, total_memory};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{available_memory, total_memory};
