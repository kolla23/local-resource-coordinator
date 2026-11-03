//! The platform code behind `sys` and the mode helpers of `paths`, which keep the signatures
//! and forward here: one file per platform, and `unix` for shared Unix code.

mod unix;
#[cfg(not(target_os = "linux"))]
pub use unix::group_usage;
pub use unix::{
    boot_id, boot_time_secs, chain_from, check_state_dir, clock_text, ensure_dir, group_members,
    job_pid_alive, make_private, near_stamp_text, own_pid_alive, pid_alive, pid_namespace,
    pid_namespace_identity, process_exe, process_is_zombie, process_start_token, process_uids,
    rfc3339, same_process_start, stamp_text, stdin_is_terminal, submitter_chain, terminal_size,
    zombie_group_leader,
};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{available_memory, group_usage, memory_pressure, process_info, total_memory};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{available_memory, process_info, total_memory};
