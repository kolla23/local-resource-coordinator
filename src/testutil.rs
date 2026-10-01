// Modified by the local-resource-coordinator fork, 2026-10-01: add peers_env (issue #5).
//! Test-only helpers.

use std::sync::{Mutex, MutexGuard, OnceLock};

/// Gives a lock for the tests that change the process environment.
///
/// The function `std::env::set_var` changes data that all the threads share.
/// The test program starts many tests together. Without this lock, one test can
/// read a variable that a different test set. That failure occurs only when the
/// machine is busy, and it looks like a fault in the program.
pub fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    // A poisoned lock shows that a different test stopped with a panic. The
    // environment is still usable. A failure of each subsequent test hides the
    // first failure.
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Sets an environment variable for one scope.
///
/// This type puts back the initial value at the end of the scope. It also puts
/// back the initial value if the test stops with a panic.
pub struct EnvVar {
    key: String,
    previous: Option<std::ffi::OsString>,
}

impl EnvVar {
    pub fn set(key: &str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self {
            key: key.to_string(),
            previous,
        }
    }

    pub fn unset(key: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::remove_var(key);
        Self {
            key: key.to_string(),
            previous,
        }
    }
}

impl Drop for EnvVar {
    fn drop(&mut self) {
        match &self.previous {
            Some(v) => std::env::set_var(&self.key, v),
            None => std::env::remove_var(&self.key),
        }
    }
}

/// Holds the environment lock and unsets `QEX_PEERS_DIR` for one test.
///
/// A test that reads the peer directory must not run while a different test
/// sets `QEX_PEERS_DIR` or `TMPDIR`: the reader then reads the directory of
/// the other test and counts no peer. The tests that set those variables hold
/// `env_lock`, so a reader must hold it too.
pub struct PeersEnv {
    // Fields drop in order: the variable is put back while the lock is held.
    _var: EnvVar,
    _lock: MutexGuard<'static, ()>,
}

/// Gives a `PeersEnv` for a test that reads the peer directory.
pub fn peers_env() -> PeersEnv {
    let lock = env_lock();
    PeersEnv {
        _var: EnvVar::unset(crate::peers::DIR_VARIABLE),
        _lock: lock,
    }
}
