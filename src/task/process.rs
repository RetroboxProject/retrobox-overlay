//! The installer's child process, run in a pseudo-terminal.
//!
//! The installer gets a real terminal (a pty), so whatever asks the user
//! something (sudo's password prompt, a git credential...) works as in any
//! terminal emulator: the screen writes the keys to the pty and shows what
//! comes back, echo included. `setsid --ctty` makes the pty the controlling
//! terminal of the installer, which sudo needs to read the password from
//! `/dev/tty` (and it keeps this crate free of `unsafe`, which a `pre_exec`
//! would need).
//!
//! One thread waits on the pty (`poll`, so it sleeps until there is output),
//! copies everything to a log file and hands the text to the event loop
//! through a channel. When the installer is gone, what is left in the pty is
//! still read, but a leftover child of it holding the pty open does not keep
//! the thread waiting.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use calloop::channel::Sender;
use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::pty::{Winsize, openpty};
use nix::sys::signal::{SigSet, Signal, killpg};
use nix::unistd::Pid;

/// What the installer's thread tells the event loop.
#[derive(Debug)]
pub enum Event {
    /// Text the installer printed.
    Output(String),
    /// The installer ended with this status (a signal's number plus 128).
    Exited(i32),
}

/// No colours or cursor tricks (the terminal box only understands plain
/// text); apt's debconf questions cannot be drawn either, so they take their
/// defaults.
const ENVIRONMENT: [(&str, &str); 4] = [
    ("NO_COLOR", "1"),
    ("TERM", "dumb"),
    ("PYTHONUNBUFFERED", "1"),
    ("DEBIAN_FRONTEND", "noninteractive"),
];
const READ_SIZE: usize = 65536;
/// How often the thread looks whether the installer has ended, when there
/// is no output to wake it.
const POLL_EVERY_MS: u8 = 100;
/// How long an installer has to stop after SIGTERM before it is killed.
const TERMINATE_WAIT: Duration = Duration::from_secs(5);

/// A running installer.
pub struct Process {
    /// The pty's master, to type into it.
    writer: File,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Process {
    /// Start `command` in a `size` (columns, rows) pty, saving its output to
    /// `log`; `sender` gets what happens.
    pub fn start(
        command: &[String],
        log: &Path,
        size: (u16, u16),
        sender: Sender<Event>,
    ) -> io::Result<Self> {
        let (program, arguments) = command
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no command"))?;
        let pty = openpty(
            Some(&Winsize {
                ws_row: size.1,
                ws_col: size.0,
                ws_xpixel: 0,
                ws_ypixel: 0,
            }),
            None,
        )
        .map_err(io::Error::from)?;
        // reading goes through `poll`: the descriptor never blocks
        fcntl(pty.master.as_fd(), FcntlArg::F_SETFL(OFlag::O_NONBLOCK)).map_err(io::Error::from)?;

        let child = spawn(program, arguments, pty.slave)?;
        let master = File::from(pty.master);
        let writer = master.try_clone()?;
        let log = open_log(log);
        let stop = Arc::new(AtomicBool::new(false));
        let thread = std::thread::Builder::new()
            .name("installer-output".into())
            .spawn({
                let stop = Arc::clone(&stop);
                move || pump(child, master, log, &sender, &stop)
            })?;
        Ok(Self {
            writer,
            stop,
            thread: Some(thread),
        })
    }

    /// Send keyboard input to the installer (ignored once it is gone).
    pub fn write(&self, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        // a full pty (nobody reads it) drops the keys instead of blocking
        let _ = (&self.writer).write_all(data);
    }
}

impl Drop for Process {
    /// Stop the installer and everything it started, and wait for that.
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Run the installer with the pty as its terminal.
fn spawn(program: &str, arguments: &[String], slave: OwnedFd) -> io::Result<Child> {
    let slave = File::from(slave);
    let mut command = if let Some(setsid) = which("setsid") {
        let mut command = Command::new(setsid);
        command.args(["--ctty", "--", program]);
        command
    } else {
        // no controlling terminal, but still a group of its own
        let mut command = Command::new(program);
        command.process_group(0);
        command
    };
    command
        .args(arguments)
        .envs(ENVIRONMENT)
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave));
    // A child starts with the signal mask of the thread that spawns it, and
    // the event loop blocks SIGTERM and SIGINT (it reads them as events):
    // an installer that cannot be told to stop would be killed after the
    // wait. A thread of its own, which unblocks them, starts it.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                if let Err(err) = SigSet::all().thread_unblock() {
                    log::warn!("cannot unblock signals for the installer: {err}");
                }
                command.spawn()
            })
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the spawning thread panicked")))
    })
}

fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(name))
            .find(|path| path.is_file())
    })
}

fn open_log(path: &Path) -> Option<File> {
    path.parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| File::create(path))
        .map_err(|err| log::warn!("cannot write {}: {err}", path.display()))
        .ok()
}

/// Thread body: pty -> log file + event loop, then the exit status.
fn pump(
    mut child: Child,
    mut master: File,
    mut log: Option<File>,
    sender: &Sender<Event>,
    stop: &AtomicBool,
) {
    let group = Pid::from_raw(i32::try_from(child.id()).unwrap_or(0));
    let mut buffer = vec![0_u8; READ_SIZE];
    let mut carry = Vec::new();
    let mut terminating: Option<Instant> = None;
    let mut ended = false;
    loop {
        if stop.load(Ordering::Relaxed) && terminating.is_none() && !ended {
            let _ = killpg(group, Signal::SIGTERM);
            terminating = Some(Instant::now());
        }
        if terminating.is_some_and(|since| since.elapsed() >= TERMINATE_WAIT) {
            let _ = killpg(group, Signal::SIGKILL);
        }
        if !ended && child.try_wait().ok().flatten().is_some() {
            ended = true;
        }
        // once the installer is gone, only what is already in the pty
        let timeout = if ended {
            PollTimeout::ZERO
        } else {
            PollTimeout::from(POLL_EVERY_MS)
        };
        let mut fds = [PollFd::new(master.as_fd(), PollFlags::POLLIN)];
        let ready = poll(&mut fds, timeout).unwrap_or(0) > 0;
        if !ready {
            if ended {
                break;
            }
            continue;
        }
        match master.read(&mut buffer) {
            // EIO: nobody has the pty open any more
            Ok(0) | Err(_) if ended => break,
            Ok(0) => {
                // the slave side is closed: the installer is on its way out
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(count) => {
                let chunk = &buffer[..count];
                if let Some(file) = log.as_mut() {
                    let _ = file.write_all(chunk);
                }
                let text = decode(&mut carry, chunk);
                if !text.is_empty() && sender.send(Event::Output(text)).is_err() {
                    break;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    // a leftover child holding the pty open does not matter now; the
    // installer itself has to be reaped for its status
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(_) => return,
        }
    };
    let code = status.code().unwrap_or_else(|| {
        std::os::unix::process::ExitStatusExt::signal(&status).map_or(1, |signal| 128 + signal)
    });
    let _ = sender.send(Event::Exited(code));
}

/// UTF-8 text from a chunk of bytes: a character cut at the end of the
/// chunk waits in `carry` for the next one; invalid bytes become U+FFFD.
fn decode(carry: &mut Vec<u8>, chunk: &[u8]) -> String {
    carry.extend_from_slice(chunk);
    let mut text = String::new();
    loop {
        match std::str::from_utf8(carry) {
            Ok(valid) => {
                text.push_str(valid);
                carry.clear();
                return text;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                text.push_str(&String::from_utf8_lossy(&carry[..valid]));
                if let Some(bad) = err.error_len() {
                    text.push('\u{fffd}');
                    carry.drain(..valid + bad);
                } else {
                    // an incomplete character at the end
                    carry.drain(..valid);
                    return text;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use calloop::channel::channel;

    #[test]
    fn decodes_a_character_cut_between_chunks() {
        let mut carry = Vec::new();
        let bytes = "añ€".as_bytes();
        let mut text = String::new();
        for chunk in bytes.chunks(2) {
            text.push_str(&decode(&mut carry, chunk));
        }
        assert_eq!(text, "añ€");
        assert_eq!(carry, Vec::<u8>::new());
    }

    #[test]
    fn replaces_invalid_bytes() {
        let mut carry = Vec::new();
        assert_eq!(decode(&mut carry, b"a\xffb"), "a\u{fffd}b");
    }

    fn run(command: &[&str]) -> (String, i32, std::path::PathBuf) {
        let log = std::env::temp_dir().join(format!(
            "retrobox-overlay-task-{}-{}.log",
            std::process::id(),
            command.join("_").len()
        ));
        let (sender, receiver) = channel();
        let command: Vec<String> = command.iter().map(|part| (*part).to_owned()).collect();
        let process = Process::start(&command, &log, (80, 24), sender).unwrap();
        let (mut text, mut status) = (String::new(), None);
        // calloop's channel is a plain std channel with a wake-up: drain it
        // with a tiny loop
        let mut event_loop = calloop::EventLoop::<(String, Option<i32>)>::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(receiver, |event, (), state| {
                if let calloop::channel::Event::Msg(event) = event {
                    match event {
                        Event::Output(output) => state.0.push_str(&output),
                        Event::Exited(code) => state.1 = Some(code),
                    }
                }
            })
            .unwrap();
        let mut state = (String::new(), None);
        for _ in 0..200 {
            event_loop
                .dispatch(Duration::from_millis(50), &mut state)
                .unwrap();
            if state.1.is_some() {
                break;
            }
        }
        drop(process);
        std::mem::swap(&mut text, &mut state.0);
        status = state.1.or(status);
        (text, status.unwrap_or(-1), log)
    }

    #[test]
    fn runs_a_command_and_logs_its_output() {
        let (text, status, log) = run(&["sh", "-c", "echo hello; exit 3"]);
        assert!(text.contains("hello"), "{text:?}");
        assert_eq!(status, 3);
        assert!(
            std::fs::read_to_string(&log)
                .unwrap_or_default()
                .contains("hello")
        );
        let _ = std::fs::remove_file(log);
    }

    #[test]
    fn the_installer_has_a_terminal() {
        let (text, status, log) = run(&["sh", "-c", "test -t 0 && echo tty"]);
        assert!(text.contains("tty"), "{text:?}");
        assert_eq!(status, 0);
        let _ = std::fs::remove_file(log);
    }

    #[test]
    fn a_missing_program_is_an_error() {
        let (sender, _receiver) = channel();
        let log = std::env::temp_dir().join("retrobox-overlay-task-missing.log");
        let command = vec!["/nonexistent/installer".to_owned()];
        // with setsid the error shows up as the exit status instead
        if which("setsid").is_none() {
            assert!(Process::start(&command, &log, (80, 24), sender).is_err());
        }
    }
}
