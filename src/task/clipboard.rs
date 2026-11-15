//! Copy text to the clipboard with whatever tool the session has.
//!
//! The overlay has no clipboard of its own to offer (it would have to keep a
//! data source alive after it exits), so the usual helpers do it: `wl-copy`
//! on Wayland, `xclip` or `xsel` on X11. They keep the text on offer in a
//! background process of their own.

use std::io::Write;
use std::process::{Command, Stdio};

/// The helpers to try, the one of the running session's kind first.
fn helpers(wayland: bool) -> Vec<(&'static str, &'static [&'static str])> {
    let mut all: Vec<(&str, &[&str])> = vec![
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];
    if !wayland {
        all.rotate_left(1);
    }
    all
}

/// Put `text` on the clipboard, off the calling thread; the helper that
/// worked is logged.
pub fn copy(text: String, wayland: bool) {
    std::thread::spawn(move || {
        for (program, arguments) in helpers(wayland) {
            let Ok(mut child) = Command::new(program)
                .args(arguments)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            let written = child
                .stdin
                .take()
                .is_some_and(|mut stdin| stdin.write_all(text.as_bytes()).is_ok());
            let worked = child.wait().is_ok_and(|status| status.success()) && written;
            if worked {
                log::info!("copied {} bytes with {program}", text.len());
                return;
            }
        }
        log::warn!("no clipboard tool (wl-copy, xclip or xsel) copied the text");
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sessions_own_helper_goes_first() {
        assert_eq!(helpers(true)[0].0, "wl-copy");
        assert_eq!(helpers(false)[0].0, "xclip");
        assert_eq!(helpers(false).len(), 3);
    }
}
