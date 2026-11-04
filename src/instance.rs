//! One overlay at a time: a second start asks the running one to close.
//!
//! The lock is `overlay.lock` in Retrobox's runtime directory, held with
//! `flock` for the whole life of the overlay (released by the kernel even if
//! it crashes) and holding its pid. Shift+F7 or the hotkey pressed again runs
//! `retrobox-overlay` a second time, which sends SIGUSR1 to that pid.

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, Write};
use std::path::Path;

use anyhow::{Context, Result};
use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;

pub const LOCK_FILE: &str = "overlay.lock";

/// What starting the overlay found.
pub enum Instance {
    /// This process is the overlay; keep the guard while it runs.
    First(Flock<File>),
    /// Another overlay runs, with this pid.
    Running(Option<i32>),
}

/// Take the lock, or report the overlay that holds it.
pub fn acquire(runtime_dir: &Path) -> Result<Instance> {
    fs::create_dir_all(runtime_dir)
        .with_context(|| format!("cannot create {}", runtime_dir.display()))?;
    let path = runtime_dir.join(LOCK_FILE);
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
            let mut text = String::new();
            file.read_to_string(&mut text).ok();
            Ok(Instance::Running(text.trim().parse().ok()))
        }
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
