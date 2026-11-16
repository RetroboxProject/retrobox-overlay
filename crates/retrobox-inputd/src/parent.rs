//! The process the daemon lives with: Retrobox's.
//!
//! Retrobox stops the daemon when it ends, but if it is killed or crashes
//! there is no one to do it, and a daemon left behind would keep mapping
//! controllers (and opening the overlay) for a Retrobox that is gone. So it
//! is told whose daemon it is (`--parent PID`, or `RETROBOX_PARENT_PID` in
//! the environment, the way systemd passes it) and stops when that process
//! ends: a pidfd turns readable then (Linux 5.3 and later); without pidfds
//! it looks every few seconds.

use std::time::Duration;

use anyhow::{Result, anyhow};
use calloop::generic::Generic;
use calloop::timer::{TimeoutAction, Timer};
use calloop::{Interest, LoopHandle, Mode, PostAction};
use nix::errno::Errno;
use nix::sys::signal::kill;
use nix::unistd::Pid;
use rustix::process::{PidfdFlags, pidfd_open};

/// The variable that names the parent when no argument does.
pub const PARENT_VARIABLE: &str = "RETROBOX_PARENT_PID";

/// How often a parent is looked for without a pidfd.
const POLL: Duration = Duration::from_secs(2);

/// The parent's PID, from its text (a positive number).
pub fn parse(text: &str) -> Option<i32> {
    text.trim().parse().ok().filter(|pid| *pid > 0)
}

/// Whether the process still exists.
fn alive(pid: i32) -> bool {
    // EPERM: it exists, but is someone else's (a PID reused by root's)
    matches!(kill(Pid::from_raw(pid), None), Ok(()) | Err(Errno::EPERM))
}

/// Call `gone` on the event loop when the process `pid` ends. An error when
/// it has already ended.
pub fn watch<State: 'static>(
    handle: &LoopHandle<'static, State>,
    pid: i32,
    gone: fn(&mut State),
) -> Result<()> {
    let raw = rustix::process::Pid::from_raw(pid).ok_or_else(|| anyhow!("bad PID {pid}"))?;
    match pidfd_open(raw, PidfdFlags::empty()) {
        Ok(fd) => {
            handle
                .insert_source(
                    Generic::new(fd, Interest::READ, Mode::Level),
                    move |_, _, state| {
                        gone(state);
                        Ok(PostAction::Remove)
                    },
                )
                .map_err(|err| anyhow!("cannot watch process {pid}: {}", err.error))?;
        }
        Err(rustix::io::Errno::SRCH) => return Err(anyhow!("process {pid} has already ended")),
        Err(err) => {
            log::info!("no pidfd for process {pid} ({err}): looking every {POLL:?}");
            if !alive(pid) {
                return Err(anyhow!("process {pid} has already ended"));
            }
            handle
                .insert_source(Timer::from_duration(POLL), move |_, &mut (), state| {
                    if alive(pid) {
                        TimeoutAction::ToDuration(POLL)
                    } else {
                        gone(state);
                        TimeoutAction::Drop
                    }
                })
                .map_err(|err| anyhow!("cannot watch process {pid}: {}", err.error))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use calloop::EventLoop;

    #[test]
    fn parses_a_pid() {
        assert_eq!(parse(" 1234\n"), Some(1234));
        assert_eq!(parse("0"), None);
        assert_eq!(parse("-5"), None);
        assert_eq!(parse("x"), None);
    }

    #[test]
    fn stops_when_the_parent_ends() {
        let mut child = std::process::Command::new("sleep")
            .arg("0.2")
            .spawn()
            .unwrap();
        let pid = i32::try_from(child.id()).unwrap();
        let mut event_loop: EventLoop<'static, bool> = EventLoop::try_new().unwrap();
        watch(&event_loop.handle(), pid, |ended| *ended = true).unwrap();
        let mut ended = false;
        // the pidfd turns readable when the process exits, before it is reaped
        for _ in 0..50 {
            event_loop
                .dispatch(Some(Duration::from_millis(100)), &mut ended)
                .unwrap();
            if ended {
                break;
            }
        }
        child.wait().unwrap();
        assert!(ended);
        assert!(
            watch(&event_loop.handle(), pid, |_| {}).is_err(),
            "already gone"
        );
    }
}
