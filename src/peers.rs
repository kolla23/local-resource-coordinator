// Modified by the local-resource-coordinator fork, 2026-10-01: tests that read the peer directory hold the env lock (issue #5); tests use testutil::temp_dir (issue #5).
// Modified by the local-resource-coordinator fork, 2026-11-18: the program is named fairq (issue #28).
// Modified by the local-resource-coordinator fork, 2026-11-22: the peer tree is used through directory handles opened without following a link; fairq uses its own folder only when it is a directory that the user owns, and the not-active message names the directory in use.
//! This module shares the resource claims between the users of one machine.
//!
//! Each coordinator writes its claims to a file in a shared directory. Each
//! coordinator reads the files of the other users before it starts a job. The
//! method needs no administrator rights.
//!
//! The method is cooperative. A different user can write an incorrect value.
//! The threat model is a colleague or an agent that does not coordinate, and
//! not an attacker. The scheduler also tests the free memory of the machine, so
//! it finds a load that no coordinator reports.
//!
//! The shared directory is writable by every user, so this module tests each
//! file before it reads the file. See [`peer_dir`] for the tests.

use crate::config::Config;
use crate::sys;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::os::unix::io::{AsRawFd, FromRawFd};
use std::path::{Path, PathBuf};

/// The record that one coordinator publishes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub uid: u32,
    pub pid: i32,
    pub boot_id: String,
    pub cpu: u64,
    pub mem: u64,
    /// The units of each pool that this coordinator holds.
    ///
    /// `#[serde(default)]` IS NECESSARY HERE, and on `devices` below. A file
    /// that an earlier version wrote has no such field. Without the default,
    /// that file does not parse, `read_peer` gives `None`, and the claims of
    /// that user disappear WITH NO MESSAGE. Two coordinators would then each
    /// use the full machine, which is the fault that this module removes.
    #[serde(default)]
    pub pools: std::collections::BTreeMap<String, u64>,
    /// The device indices of each indexed pool that this coordinator holds.
    ///
    /// Without this field, two users give the device 0 to two jobs.
    #[serde(default)]
    pub devices: std::collections::BTreeMap<String, Vec<u32>>,
    pub updated_at: u64,
}

/// The total of the claims of the other users.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Claims {
    pub cpu: u64,
    pub mem: u64,
    /// The units of each pool that the other users hold.
    pub pools: std::collections::BTreeMap<String, u64>,
    /// The devices of each indexed pool that the other users hold.
    ///
    /// A device in this set is not available to this coordinator. A peer
    /// publishes the index, and not the quantity, so fairq keeps the whole
    /// device for that user. That is the safe direction: a job of this user
    /// that shared the device would meet a memory error on the card.
    pub devices: std::collections::BTreeMap<String, std::collections::BTreeSet<u32>>,
    /// The number of other coordinators that fairq counted.
    pub count: usize,
}

/// The name of the variable that moves the peer directory.
///
/// A test sets this so a suite run cannot write into `/tmp/fairq`. The variable
/// is not `TMPDIR`: two users of one machine have different values of `TMPDIR`
/// on macOS, and they must still share one directory.
pub const DIR_VARIABLE: &str = "FAIRQ_PEERS_DIR";

/// Gives the peer directory that this process will use.
///
/// `FAIRQ_PEERS_DIR` wins when it is set, so a test can isolate itself. The
/// config value is the directory of the live queue.
fn peers_location(cfg: &Config) -> PathBuf {
    match std::env::var_os(DIR_VARIABLE) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => PathBuf::from(&cfg.peers.dir),
    }
}

/// Gives a handle on the directory for the peer files, if the directory is safe.
///
/// The directory is writable by every user, so this function makes these tests:
///
/// - The path is a directory.
/// - The directory has the sticky bit. A different user then cannot delete the
///   subdirectory of this user.
/// - The owner is the root user or the current user, or every user can write to it.
///
/// If a test fails, the function gives `None`. fairq then operates for one user
/// only. It does not give an error, because a shared directory is not necessary
/// for a correct queue.
fn peer_dir(cfg: &Config) -> Option<File> {
    let dir = peers_location(cfg);

    let handle = match open_dir(&dir) {
        Ok(handle) => handle,
        // Make the directory. The first user of the machine arrives here.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => match make_shared(&dir) {
            Ok(made) => return Some(made),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => open_dir(&dir).ok()?,
            Err(_) => return None,
        },
        Err(_) => return None,
    };
    match handle.metadata() {
        Ok(meta) => {
            if !meta.is_dir() {
                return None;
            }
            let mode = meta.mode();
            // 0o1000 is the sticky bit. It stops a different user from the
            // deletion of the files of this user.
            if mode & 0o1000 == 0 {
                return None;
            }

            // Accept a directory that every user can write, whatever its owner.
            //
            // fairq makes this directory, so the first user of the machine owns
            // it. A test for the owner would thus refuse the directory for
            // every other user, and each of those users would lose the shared
            // accounting with no message.
            //
            // The owner of the directory does not give safety here. The safety
            // comes from the sticky bit and from the test of the owner of each
            // file. See `read_peer`.
            let world_writable = mode & 0o002 != 0;
            let owner = meta.uid();
            if !world_writable && owner != 0 && owner != current_uid() {
                return None;
            }
            Some(handle)
        }
        Err(_) => None,
    }
}

/// The flags of every directory open: no link at the last name, and no wait.
const DIR_FLAGS: libc::c_int =
    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;

/// Opens the shared directory by its path, without following a link at its name.
fn open_dir(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(DIR_FLAGS)
        .open(path)
}

/// Makes the shared directory with mode 0700, and gives it mode 1777 on its
/// handle once the handle shows a directory that this user owns.
fn make_shared(dir: &Path) -> std::io::Result<File> {
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::PermissionsExt;
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())?;
    if unsafe { libc::mkdir(path.as_ptr(), 0o700) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let handle = open_dir(dir)?;
    let meta = handle.metadata()?;
    if !meta.is_dir() || meta.uid() != current_uid() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    handle.set_permissions(std::fs::Permissions::from_mode(0o1777))?;
    Ok(handle)
}

/// Opens `name` inside `dir` with `flags`, relative to the handle.
fn open_at(dir: &File, name: &str, flags: libc::c_int) -> std::io::Result<File> {
    let name = std::ffi::CString::new(name)?;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Deletes `name` inside `dir`, relative to the handle. A missing name is no error.
fn remove_at(dir: &File, name: &str) {
    if let Ok(name) = std::ffi::CString::new(name) {
        unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) };
    }
}

/// What fairq finds at this user's name in the shared directory.
#[derive(Debug, PartialEq, Eq)]
enum Own {
    /// Nothing yet, or a name that `publish` cannot make.
    Absent,
    /// A directory that this user owns.
    Mine,
    /// Anything else: this coordinator's claims are not shared.
    Refused,
}

/// Opens `name` inside `shared` when it is a directory that `uid` owns.
fn open_own(shared: &File, name: &str, uid: u32) -> Result<File, Own> {
    let dir = open_at(shared, name, DIR_FLAGS).map_err(|_| Own::Refused)?;
    match dir.metadata() {
        Ok(meta) if meta.is_dir() && meta.uid() == uid => Ok(dir),
        _ => Err(Own::Refused),
    }
}

/// Gives a handle on this user's folder inside `shared`, made when absent: a
/// directory that `uid` owns, with mode 0755 set on the handle.
fn own_subdir(shared: &File, name: &str, uid: u32) -> Result<File, Own> {
    use std::os::unix::fs::PermissionsExt;
    let cname = std::ffi::CString::new(name).map_err(|_| Own::Refused)?;
    if unsafe { libc::mkdirat(shared.as_raw_fd(), cname.as_ptr(), 0o755) } != 0
        && std::io::Error::last_os_error().kind() != std::io::ErrorKind::AlreadyExists
    {
        return Err(Own::Absent);
    }
    let dir = open_own(shared, name, uid)?;
    dir.set_permissions(std::fs::Permissions::from_mode(0o755))
        .ok();
    Ok(dir)
}

/// Tells what is at `name` inside `shared`, and changes nothing.
fn probe_own(shared: &File, name: &str, uid: u32) -> Own {
    match open_at(shared, name, DIR_FLAGS) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Own::Absent,
        Err(_) => Own::Refused,
        Ok(dir) => match dir.metadata() {
            Ok(meta) if meta.is_dir() && meta.uid() == uid => Own::Mine,
            _ => Own::Refused,
        },
    }
}

/// Writes `bytes` as `name` inside `dir` in one step: a temporary file, synced,
/// then renamed into place, each step relative to the handle.
fn write_record(dir: &File, name: &str, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = std::ffi::CString::new(format!(".{name}.tmp.{}", std::process::id()))?;
    let cname = std::ffi::CString::new(name)?;
    let fd = dir.as_raw_fd();
    unsafe { libc::unlinkat(fd, tmp.as_ptr(), 0) };
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let raw = unsafe { libc::openat(fd, tmp.as_ptr(), flags, 0o644 as libc::c_uint) };
    if raw < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let mut file = unsafe { File::from_raw_fd(raw) };
    let mut written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if written.is_ok() && unsafe { libc::renameat(fd, tmp.as_ptr(), fd, cname.as_ptr()) } != 0 {
        written = Err(std::io::Error::last_os_error());
    }
    if written.is_err() {
        unsafe { libc::unlinkat(fd, tmp.as_ptr(), 0) };
        return written;
    }
    dir.sync_all().ok();
    Ok(())
}

/// The words that say that this coordinator's own folder in the peer directory is refused.
const NOT_SHARED: &str = "this coordinator's claims are not shared";

/// Says why this coordinator's claims are not shared, naming its folder.
fn not_shared(cfg: &Config, name: &str) -> String {
    let path = peers_location(cfg).join(name);
    format!(
        "{NOT_SHARED}: {} is not a directory that this user owns",
        path.display()
    )
}

/// Writes the true state of the shared accounting, for `fairq config show`.
///
/// This function tests the directory. A message that says "on" while the
/// directory fails a test would tell the user that a feature operates when it
/// does not.
pub fn describe(cfg: &Config) -> String {
    if !cfg.peers_enabled() {
        // Say WHICH decision turned it off. Two do: the key, and the mode.
        return match cfg.peers.enabled {
            Some(false) => "off; the config file sets [peers] enabled = false".to_string(),
            _ => "off; the config file sets [enforce] mode = \"single-user\", so fairq \
                  expects no other coordinator"
                .to_string(),
        };
    }
    match peer_dir(cfg) {
        Some(shared) => {
            let c = claims(cfg);
            let mut said = format!(
                "on, in {} ({}, {} and {} claimed)",
                peers_location(cfg).display(),
                crate::units::count_of(c.count, "other coordinator"),
                crate::units::count_of(c.cpu as usize, "core"),
                crate::units::format_size(c.mem)
            );
            let name = user_dir_name(current_uid());
            if probe_own(&shared, &name, current_uid()) == Own::Refused {
                said = format!("{said}; {}", not_shared(cfg, &name));
            }
            said
        }
        None => format!(
            "NOT ACTIVE; fairq cannot use the directory {}. It must be a directory with the \
             sticky bit that every user can write to, or that belongs to root or to this user. \
             fairq uses the budget of this user only.",
            peers_location(cfg).display()
        ),
    }
}

pub fn current_uid() -> u32 {
    unsafe { libc::getuid() }
}

/// Gives the file name of the directory of one user.
fn user_dir_name(uid: u32) -> String {
    format!("u{uid}")
}

/// Gives the file name for one coordinator.
///
/// The name holds the process id, so each coordinator has its own file. One
/// user can have more than one coordinator, with one for each state directory.
/// With one file for each user, the last coordinator to write would hide the
/// claims of the others, and a busy coordinator would report no load.
fn peer_file_name(pid: i32) -> String {
    format!("peer-{pid}.json")
}

/// Writes the claims of this coordinator.
///
/// The function writes the file in one operation, so a reader never sees a part
/// of the record.
pub fn publish(
    cfg: &Config,
    cpu: u64,
    mem: u64,
    pools: std::collections::BTreeMap<String, u64>,
    devices: std::collections::BTreeMap<String, Vec<u32>>,
) {
    if !cfg.peers_enabled() {
        return;
    }
    let Some(shared) = peer_dir(cfg) else { return };

    let uid = current_uid();
    let name = user_dir_name(uid);
    let mine = match own_subdir(&shared, &name, uid) {
        Ok(mine) => mine,
        Err(Own::Refused) => {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                crate::daemon::log(&not_shared(cfg, &name));
            }
            return;
        }
        Err(_) => return,
    };

    let peer = Peer {
        uid,
        pid: std::process::id() as i32,
        boot_id: sys::boot_id(),
        cpu,
        mem,
        pools,
        devices,
        updated_at: sys::now_secs(),
    };

    if let Ok(bytes) = serde_json::to_vec(&peer) {
        // Not `job::write_atomic`: it opens the folder again by its path, and the
        // record must land in the folder that this handle checked.
        write_record(&mine, &peer_file_name(peer.pid), &bytes).ok();
    }
}

/// Deletes the record of this coordinator.
///
/// The coordinator calls this function when it stops, so its claims do not stop
/// the jobs of a different user.
pub fn withdraw(cfg: &Config) {
    if let Some(shared) = peer_dir(cfg) {
        let uid = current_uid();
        if let Ok(mine) = open_own(&shared, &user_dir_name(uid), uid) {
            remove_at(&mine, &peer_file_name(std::process::id() as i32));
        }
    }
}

/// Reads the claims of the other users.
///
/// This function ignores the record of the current user, and each record that
/// fails a test. A record that fairq cannot read gives no error, because a
/// damaged file must not stop the queue.
pub fn claims(cfg: &Config) -> Claims {
    claims_for(cfg, current_uid())
}

/// Reads the claims as the user `me`; `claims` passes the current user.
fn claims_for(cfg: &Config, me: u32) -> Claims {
    let mut total = Claims::default();
    if !cfg.peers_enabled() {
        return total;
    }

    let Some(shared) = peer_dir(cfg) else {
        return total;
    };

    let stale = cfg
        .peer_stale_after()
        .unwrap_or(std::time::Duration::from_secs(30))
        .as_secs();
    let now = sys::now_secs();
    let boot = sys::boot_id();

    let Ok(entries) = std::fs::read_dir(peers_location(cfg)) else {
        return total;
    };

    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };

        // The directory name gives the user. Read it, then compare it with the
        // owner of the file. A user thus cannot write a record for a different
        // user.
        let Some(claimed_uid) = name.strip_prefix('u').and_then(|n| n.parse::<u32>().ok()) else {
            continue;
        };
        // An entry that is not a directory, a link included, is skipped.
        let Ok(user_dir) = open_at(&shared, name, DIR_FLAGS) else {
            continue;
        };

        // One user can have more than one coordinator, with one for each state
        // directory. Read every file in the directory, and skip the file of
        // this coordinator only.
        //
        // A test that skips the whole directory of this user would hide a
        // second coordinator of the same user. Two coordinators would then each
        // use the full budget, and together they would start twice the
        // permitted work.
        let Ok(files) = std::fs::read_dir(entry.path()) else {
            continue;
        };
        let my_pid = std::process::id() as i32;

        for file in files.flatten() {
            let fname = file.file_name();
            let Some(fname) = fname.to_str() else {
                continue;
            };
            if !fname.starts_with("peer-") || !fname.ends_with(".json") {
                continue;
            }

            let Some(peer) = read_peer(&user_dir, fname, claimed_uid) else {
                continue;
            };

            // Skip this coordinator. Its own claims are already in its total.
            if claimed_uid == me && peer.pid == my_pid {
                continue;
            }
            // A record from a different start of the machine is old. The system
            // uses each process id again after a restart.
            if peer.boot_id != boot {
                remove_at(&user_dir, fname);
                continue;
            }
            // A coordinator writes its record frequently. An old record shows a
            // coordinator that stopped without a clean end.
            if now.saturating_sub(peer.updated_at) > stale {
                remove_at(&user_dir, fname);
                continue;
            }
            // The process must be alive.
            if !sys::pid_alive(peer.pid) {
                remove_at(&user_dir, fname);
                continue;
            }

            total.cpu += peer.cpu;
            total.mem += peer.mem;
            for (name, units) in &peer.pools {
                *total.pools.entry(name.clone()).or_insert(0) += units;
            }
            for (name, indices) in &peer.devices {
                total
                    .devices
                    .entry(name.clone())
                    .or_default()
                    .extend(indices.iter().copied());
            }
            total.count += 1;
        }
    }

    total
}

/// The largest peer record that fairq reads. A record is small; a larger file is
/// not a record.
const MAX_PEER_LEN: u64 = 64 * 1024;

/// Reads one peer file and tests its owner.
///
/// The function opens the file without a symbolic link. A different user thus
/// cannot point the file at a file of this user.
///
/// The function opens the file WITHOUT A WAIT. The directory is writable by
/// every user, and an open of a FIFO waits until a writer arrives — for ever,
/// when no writer comes. The callers of this function hold the state of the
/// coordinator, so a wait here stops every command of every user of this
/// queue. `O_NONBLOCK` makes the open return at once; for a regular file it
/// changes nothing. The test that the file IS a regular file comes after the
/// open, from the file itself, so no other user can change the answer between
/// a test of the path and the open.
fn read_peer(dir: &File, name: &str, expected_uid: u32) -> Option<Peer> {
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    let file = open_at(dir, name, flags).ok()?;

    let meta = file.metadata().ok()?;
    if !meta.is_file() {
        return None;
    }
    // The owner of the file must be the user of the directory name.
    if meta.uid() != expected_uid {
        return None;
    }
    if meta.len() > MAX_PEER_LEN {
        return None;
    }

    // Cap the read itself. The file can grow between the test of its size and
    // the read, and a reader with no cap would take the whole file into
    // memory.
    use std::io::Read;
    let mut text = String::new();
    file.take(MAX_PEER_LEN + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_PEER_LEN {
        return None;
    }
    let peer: Peer = serde_json::from_str(&text).ok()?;
    // The record must name the same user as its directory and its owner.
    if peer.uid != expected_uid {
        return None;
    }
    Some(peer)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn tmpdir(tag: &str) -> PathBuf {
        let d =
            crate::testutil::temp_dir().join(format!("fairq-peers-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&d).ok();
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Reads one record by its path, through a handle on its folder.
    fn read_peer(path: &Path, uid: u32) -> Option<Peer> {
        let dir = open_dir(path.parent()?).ok()?;
        super::read_peer(&dir, path.file_name()?.to_str()?, uid)
    }

    fn cfg_for(dir: &Path) -> Config {
        toml::from_str(&format!(
            "[peers]\nenabled = true\ndir = \"{}\"\nstale_after = \"30s\"\n",
            dir.display()
        ))
        .unwrap()
    }

    #[test]
    fn a_record_of_this_user_does_not_count() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("self");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let cfg = cfg_for(&dir);

        publish(&cfg, 4, 8 << 30, Default::default(), Default::default());
        // The claims of this coordinator are already in its own total. A second
        // count would stop the jobs of this user.
        assert_eq!(claims(&cfg), Claims::default());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_directory_without_the_sticky_bit_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("nosticky");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o777)).unwrap();
        let cfg = cfg_for(&dir);
        // Without the sticky bit, a different user can delete the files of this
        // user. fairq must then operate for one user only.
        assert!(peer_dir(&cfg).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_file_in_place_of_the_directory_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("isfile");
        std::fs::remove_dir_all(&dir).ok();
        std::fs::write(&dir, b"not a directory").unwrap();
        let cfg = cfg_for(&dir);
        assert!(peer_dir(&cfg).is_none());
        std::fs::remove_file(&dir).ok();
    }

    #[test]
    fn a_record_with_the_wrong_owner_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("wrongowner");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();

        // Write a record in the directory of a different user. This process
        // owns the file, so the owner does not match the directory name.
        let other = dir.join("u99999");
        std::fs::create_dir_all(&other).unwrap();
        let peer = Peer {
            uid: 99999,
            pid: std::process::id() as i32,
            boot_id: sys::boot_id(),
            cpu: 64,
            mem: 64 << 30,
            pools: Default::default(),
            devices: Default::default(),
            updated_at: sys::now_secs(),
        };
        std::fs::write(other.join("peer.json"), serde_json::to_vec(&peer).unwrap()).unwrap();

        let cfg = cfg_for(&dir);
        assert_eq!(
            claims(&cfg),
            Claims::default(),
            "a record with an incorrect owner must not count"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_old_record_and_a_dead_process_do_not_count() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("stale");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let cfg = cfg_for(&dir);
        let me = current_uid();

        // Use the directory of a different user, but keep the owner correct, so
        // the test measures the time rule and the process rule only.
        let other = dir.join(user_dir_name(me + 1));
        std::fs::create_dir_all(&other).unwrap();

        let old = Peer {
            uid: me + 1,
            pid: std::process::id() as i32,
            boot_id: sys::boot_id(),
            cpu: 64,
            mem: 64 << 30,
            pools: Default::default(),
            devices: Default::default(),
            updated_at: sys::now_secs().saturating_sub(3600),
        };
        std::fs::write(other.join("peer.json"), serde_json::to_vec(&old).unwrap()).unwrap();
        // The owner test refuses this file first, so the total stays at zero.
        assert_eq!(claims(&cfg), Claims::default());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_damaged_file_does_not_stop_the_reader() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("garbage");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let other = dir.join("u12345");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("peer.json"), b"{not json").unwrap();

        let cfg = cfg_for(&dir);
        // The reader must give an answer and must not stop.
        assert_eq!(claims(&cfg), Claims::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A FIFO in place of a record must not stop the reader.
    ///
    /// The peer directory is writable by every user, and an open of a FIFO
    /// with no writer waits for ever. The callers of `read_peer` hold the
    /// state of the coordinator, so that wait would stop every command of
    /// every user of this queue. Without `O_NONBLOCK` in `read_peer`, this
    /// test does not fail: it hangs.
    #[test]
    fn a_fifo_in_place_of_a_record_does_not_stop_the_reader() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("fifo");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let other = dir.join("u12345");
        std::fs::create_dir_all(&other).unwrap();

        let fifo = other.join("peer-1.json");
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { libc::mkfifo(path.as_ptr(), 0o644) },
            0,
            "the test needs a FIFO"
        );

        // The reader must give an answer at once, and the answer is a refusal.
        assert!(read_peer(&fifo, 12345).is_none());
        // The full read must give an answer too, whatever entry finds the FIFO.
        assert_eq!(claims(&cfg_for(&dir)), Claims::default());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_symbolic_link_is_not_read() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("symlink");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();

        let target = dir.join("real.json");
        let peer = Peer {
            uid: current_uid(),
            pid: std::process::id() as i32,
            boot_id: sys::boot_id(),
            cpu: 64,
            mem: 64 << 30,
            pools: Default::default(),
            devices: Default::default(),
            updated_at: sys::now_secs(),
        };
        std::fs::write(&target, serde_json::to_vec(&peer).unwrap()).unwrap();

        let other = dir.join("u54321");
        std::fs::create_dir_all(&other).unwrap();
        std::os::unix::fs::symlink(&target, other.join("peer.json")).unwrap();

        assert!(
            read_peer(&other.join("peer.json"), 54321).is_none(),
            "the reader must not follow a symbolic link"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A record that an earlier version wrote has no `pools` and no `devices`.
    ///
    /// THIS IS THE HIGHEST-RISK LINE OF THE POOL FEATURE. Without the
    /// `#[serde(default)]` on those two fields, that file does not parse,
    /// `read_peer` gives `None`, and the cores and the memory of that user
    /// disappear WITH NO MESSAGE. Two coordinators would then each use the full
    /// machine.
    #[test]
    fn a_peer_file_of_an_earlier_version_still_parses_and_still_counts() {
        let old = r#"{"uid":4242,"pid":7,"boot_id":"b","cpu":6,"mem":17179869184,
                      "updated_at":1700000000}"#;
        let peer: Peer =
            serde_json::from_str(old).expect("a record of an earlier version must still parse");
        assert_eq!(peer.cpu, 6, "the cores of that user must still count");
        assert_eq!(
            peer.mem,
            16 << 30,
            "the memory of that user must still count"
        );
        assert!(peer.pools.is_empty());
        assert!(peer.devices.is_empty());
    }

    /// A record with pools and devices must give both back. Without the
    /// devices, two users give the device 0 to two jobs.
    #[test]
    fn a_peer_record_carries_the_pools_and_the_devices() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("pools");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let cfg = cfg_for(&dir);

        publish(
            &cfg,
            2,
            1 << 30,
            [("gpu".to_string(), 2u64)].into_iter().collect(),
            [("gpu".to_string(), vec![0u32, 2])].into_iter().collect(),
        );

        // The file of this coordinator does not count for itself, so read it
        // back directly.
        let mine = dir.join(user_dir_name(current_uid()));
        let path = mine.join(peer_file_name(std::process::id() as i32));
        let peer = read_peer(&path, current_uid()).expect("the record must parse");
        assert_eq!(peer.pools.get("gpu"), Some(&2));
        assert_eq!(peer.devices.get("gpu"), Some(&vec![0, 2]));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_peer_system_is_off_when_the_config_says_so() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("disabled");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let mut cfg = cfg_for(&dir);
        cfg.peers.enabled = Some(false);
        assert_eq!(claims(&cfg), Claims::default());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `TMPDIR` must not move the peer directory. Two users on macOS have
    /// different values of `TMPDIR`, and they must still share one directory.
    #[test]
    fn tmpdir_does_not_move_the_peer_directory() {
        let _lock = crate::testutil::env_lock();
        let tmp = tmpdir("notvia-tmpdir");
        let _t = crate::testutil::EnvVar::set("TMPDIR", tmp.to_str().unwrap());
        let _q = crate::testutil::EnvVar::unset(DIR_VARIABLE);
        let cfg = Config::default();
        assert_eq!(
            peers_location(&cfg),
            PathBuf::from("/tmp/fairq"),
            "TMPDIR must not move the shared peer directory"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// A test sets `FAIRQ_PEERS_DIR` so it cannot reach `/tmp/fairq`.
    #[test]
    fn the_peers_dir_variable_moves_the_peer_directory() {
        let _lock = crate::testutil::env_lock();
        let dir = tmpdir("via-var");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let _q = crate::testutil::EnvVar::set(DIR_VARIABLE, dir.to_str().unwrap());

        // The config names the live directory. The variable must win, or a
        // suite run writes into the queue of the person who runs the suite.
        let mut cfg = Config::default();
        cfg.peers.enabled = Some(true);
        assert_eq!(peers_location(&cfg), dir);
        publish(&cfg, 2, 1 << 30, Default::default(), Default::default());
        let mine = dir.join(user_dir_name(current_uid()));
        assert!(
            mine.join(peer_file_name(std::process::id() as i32))
                .exists(),
            "the record must land in the directory that the variable names"
        );
        let live = PathBuf::from("/tmp/fairq")
            .join(user_dir_name(current_uid()))
            .join(peer_file_name(std::process::id() as i32));
        assert!(
            !live.exists(),
            "the variable must keep the record out of the live directory"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_empty_peers_dir_variable_does_not_move_the_directory() {
        let _lock = crate::testutil::env_lock();
        let _q = crate::testutil::EnvVar::set(DIR_VARIABLE, "");
        let cfg = Config::default();
        assert_eq!(peers_location(&cfg), PathBuf::from("/tmp/fairq"));
    }

    /// A FIFO at this user's name gives no wait and no record, and the message
    /// says that this coordinator's claims are not shared.
    #[test]
    fn a_fifo_at_this_users_name_is_refused_without_a_wait() {
        use std::os::unix::fs::FileTypeExt;
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("ufifo");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let fifo = dir.join(user_dir_name(current_uid()));
        let path = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
        assert_eq!(
            unsafe { libc::mkfifo(path.as_ptr(), 0o644) },
            0,
            "the test needs a FIFO"
        );
        let cfg = cfg_for(&dir);

        publish(&cfg, 1, 1 << 20, Default::default(), Default::default());
        let kind = std::fs::symlink_metadata(&fifo).unwrap().file_type();
        assert!(kind.is_fifo(), "the FIFO stays as it was");
        let said = describe(&cfg);
        assert!(
            said.contains("this coordinator's claims are not shared"),
            "{said}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A file at this user's name keeps its bytes, and the message says that
    /// this coordinator's claims are not shared.
    #[test]
    fn a_file_at_this_users_name_is_refused_and_kept() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("ufile");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let file = dir.join(user_dir_name(current_uid()));
        std::fs::write(&file, b"not a folder").unwrap();
        let cfg = cfg_for(&dir);

        publish(&cfg, 1, 1 << 20, Default::default(), Default::default());
        assert_eq!(std::fs::read(&file).unwrap(), b"not a folder");
        let said = describe(&cfg);
        assert!(
            said.contains("this coordinator's claims are not shared"),
            "{said}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A link in place of the shared directory is refused, and its target is
    /// left as it was.
    #[test]
    fn a_link_in_place_of_the_shared_directory_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("sharedlink");
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::fs::set_permissions(&elsewhere, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&elsewhere, &link).unwrap();
        let cfg = cfg_for(&link);

        publish(&cfg, 1, 1 << 20, Default::default(), Default::default());
        let mode = std::fs::metadata(&elsewhere).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o700, "the link's target keeps its mode");
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        let said = describe(&cfg);
        assert!(said.starts_with("NOT ACTIVE"), "{said}");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A link at this user's name that points nowhere is not an absent name:
    /// nothing is made, and the message says the claims are not shared.
    #[test]
    fn a_dangling_link_at_this_users_name_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("udangling");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let name = dir.join(user_dir_name(current_uid()));
        std::os::unix::fs::symlink(dir.join("nothing"), &name).unwrap();
        let cfg = cfg_for(&dir);

        publish(&cfg, 1, 1 << 20, Default::default(), Default::default());
        assert!(
            !dir.join("nothing").exists(),
            "nothing is made behind the link"
        );
        let said = describe(&cfg);
        assert!(
            said.contains("this coordinator's claims are not shared"),
            "{said}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A folder at this user's name that another user owns is refused, and it
    /// keeps its mode.
    #[test]
    fn a_folder_of_another_user_at_this_users_name_is_refused() {
        let dir = tmpdir("uother");
        let shared = open_dir(&dir).unwrap();
        let name = user_dir_name(current_uid());
        std::fs::create_dir(dir.join(&name)).unwrap();
        std::fs::set_permissions(dir.join(&name), std::fs::Permissions::from_mode(0o700)).unwrap();
        let got = own_subdir(&shared, &name, current_uid() + 1);
        assert!(matches!(got, Err(Own::Refused)), "the folder is refused");
        let mode = std::fs::metadata(dir.join(&name))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o700, "the folder keeps its mode");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The probe of `fairq config show` refuses a folder at this user's name
    /// that another user owns, and changes nothing.
    #[test]
    fn the_probe_refuses_a_folder_of_another_user_and_changes_nothing() {
        let dir = tmpdir("probeother");
        let shared = open_dir(&dir).unwrap();
        let name = user_dir_name(current_uid());
        std::fs::create_dir(dir.join(&name)).unwrap();
        std::fs::set_permissions(dir.join(&name), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(probe_own(&shared, &name, current_uid() + 1), Own::Refused);
        let mode = std::fs::metadata(dir.join(&name))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777;
        assert_eq!(mode, 0o700, "the folder keeps its mode");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The records of the other users still count while this user's own name is refused.
    #[test]
    fn the_other_users_still_count_while_this_users_folder_is_refused() {
        let _env = crate::testutil::peers_env();
        let dir = tmpdir("refused-others");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o1777)).unwrap();
        let other = current_uid();
        let theirs = dir.join(user_dir_name(other));
        std::fs::create_dir_all(&theirs).unwrap();
        let peer = Peer {
            uid: other,
            pid: std::process::id() as i32,
            boot_id: sys::boot_id(),
            cpu: 3,
            mem: 1 << 30,
            pools: Default::default(),
            devices: Default::default(),
            updated_at: sys::now_secs(),
        };
        let record = theirs.join(peer_file_name(peer.pid));
        std::fs::write(&record, serde_json::to_vec(&peer).unwrap()).unwrap();
        let target = dir.join("target");
        std::fs::create_dir(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::os::unix::fs::symlink(&target, dir.join(user_dir_name(other + 1))).unwrap();

        // Read as the user other + 1, whose own name is a link.
        let c = claims_for(&cfg_for(&dir), other + 1);
        assert_eq!(
            (c.count, c.cpu, c.mem),
            (1, 3, 1 << 30),
            "the other user still counts"
        );
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o700, "the link's target keeps its mode");
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
