//! One overlay at a time, and one daemon: a second start reaches the
//! running one instead.
//!
//! The locks are `overlay.lock` (the overlay on screen) and
//! `overlay-daemon.lock` (the daemon, see `daemon.rs`) in Retrobox's runtime
//! directory, each held with `flock` for the whole life of its process
//! (released by the kernel even if it crashes) and holding its pid. A second
//! start sends SIGUSR1 to that pid: the overlay closes, the daemon opens or
//! closes the overlay.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

pub const LOCK_FILE: &str = "overlay.lock";
pub const DAEMON_LOCK_FILE: &str = "overlay-daemon.lock";

/// How long a second start waits for the first one to write its pid (it
/// takes the lock, then writes it).
const PID_WAIT: Duration = Duration::from_millis(50);

/// What starting the overlay found.
pub enum Instance {
    /// This process is the overlay; keep the guard while it runs.
    First(Flock<File>),
    /// Another overlay runs, with this pid.
    Running(Option<i32>),
}

/// Take the overlay's lock, or report the overlay that holds it.
pub fn acquire(runtime_dir: &Path) -> Result<Instance> {
    acquire_named(runtime_dir, LOCK_FILE)
}

/// Take a lock, or report the process that holds it.
pub fn acquire_named(runtime_dir: &Path, name: &str) -> Result<Instance> {
    fs::create_dir_all(runtime_dir)
        .with_context(|| format!("cannot create {}", runtime_dir.display()))?;
    let path = runtime_dir.join(name);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("cannot open {}", path.display()))?;

    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(mut lock) => {
            lock.set_len(0)?;
            lock.rewind()?;
            write!(lock, "{}", std::process::id())?;
            lock.flush()?;
            Ok(Instance::First(lock))
        }
        Err((mut file, _)) => {
            let mut pid = read_pid(&mut file);
            if pid.is_none() {
                // locked a moment ago: its pid is being written
                std::thread::sleep(PID_WAIT);
                pid = read_pid(&mut file);
            }
            Ok(Instance::Running(pid))
        }
    }
}

fn read_pid(file: &mut File) -> Option<i32> {
    let mut text = String::new();
    file.rewind().ok()?;
    file.read_to_string(&mut text).ok()?;
    text.trim().parse().ok()
}

/// Whether the daemon runs, and its pid.
pub fn daemon_pid(runtime_dir: &Path) -> Option<i32> {
    let mut file = File::open(runtime_dir.join(DAEMON_LOCK_FILE)).ok()?;
    // the daemon holds the lock: taking it fails
    match Flock::lock(file.try_clone().ok()?, FlockArg::LockSharedNonblock) {
        Ok(_) => None,
        Err(_) => read_pid(&mut file),
    }
}

/// Ask the running overlay to close.
pub fn close_running(pid: Option<i32>) {
    match pid {
        Some(pid) => {
            if let Err(err) = kill(Pid::from_raw(pid), Signal::SIGUSR1) {
                log::warn!("cannot reach the running overlay ({pid}): {err}");
            }
        }
        None => log::warn!("an overlay is running but its pid is unknown"),
    }
}
