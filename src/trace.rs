//! A fine timeline of what the overlay does, to tell on a given machine
//! where its time goes: with `RETROBOX_OVERLAY_TRACE=<file>`, every input
//! event, scroll step, frame drawn or put off, buffer handed over, configure
//! and frame callback is a line of that file (appended to: one session after
//! another, each opening with a `start` line).
//!
//! A line is `<time> <event> <field>=<value>...`; the time is
//! `CLOCK_MONOTONIC` in milliseconds: the clock of the compositor's own
//! times (input events, frame callbacks: wlroots, KWin, Weston), so that
//! the delay of each can be told, and of Python's `time.monotonic()`, which
//! `scripts/probe.py` samples the system with (`scripts/trace_report.py`
//! reads both).

use std::fmt::Arguments;
use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::sync::{Mutex, OnceLock};

use rustix::time::{ClockId, clock_gettime};

/// The variable naming the file.
const VARIABLE: &str = "RETROBOX_OVERLAY_TRACE";
/// How often the lines are written out at most (milliseconds): a write per
/// line would weigh on what is measured.
const FLUSH_EVERY: f64 = 250.0;

struct Sink {
    out: BufWriter<File>,
    flushed: f64,
}

fn sink() -> Option<&'static Mutex<Sink>> {
    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var_os(VARIABLE).filter(|path| !path.is_empty())?;
        match OpenOptions::new().create(true).append(true).open(&path) {
            Ok(file) => Some(Mutex::new(Sink {
                out: BufWriter::new(file),
                flushed: now(),
            })),
            Err(err) => {
                log::warn!("cannot trace to {}: {err}", path.display());
                None
            }
        }
    })
    .as_ref()
}

/// `CLOCK_MONOTONIC`, in milliseconds.
pub fn now() -> f64 {
    let time = clock_gettime(ClockId::Monotonic);
    time.tv_sec as f64 * 1000.0 + time.tv_nsec as f64 / 1_000_000.0
}

/// How long ago (milliseconds) a compositor's time (milliseconds of
/// `CLOCK_MONOTONIC`, wrapping at 32 bits) was.
pub fn since(time: u32) -> f64 {
    let now = now();
    let wrapped = (now as u64 % (1 << 32)) as u32;
    f64::from(wrapped.wrapping_sub(time) as i32) + now.fract()
}

/// Whether there is a trace to write (so that a field costly to work out is
/// only worked out then).
pub fn enabled() -> bool {
    sink().is_some()
}

/// Add a line (see [`trace!`]).
pub fn event(name: &str, fields: Arguments<'_>) {
    let Some(sink) = sink() else {
        return;
    };
    let Ok(mut sink) = sink.lock() else {
        return;
    };
    let now = now();
    let fields = fields.to_string();
    let separator = if fields.is_empty() { "" } else { " " };
    // a trace that cannot be written is not worth stopping for
    let _ = writeln!(sink.out, "{now:.3} {name}{separator}{fields}");
    if now - sink.flushed >= FLUSH_EVERY {
        let _ = sink.out.flush();
        sink.flushed = now;
    }
}

/// Write out what is left (at the end).
pub fn flush() {
    if let Some(sink) = sink()
        && let Ok(mut sink) = sink.lock()
    {
        let _ = sink.out.flush();
    }
}

/// A line of the trace, when there is one: `trace!("draw", "render={:.3}",
/// ms)`; the fields are only formatted then.
macro_rules! trace {
    ($name:literal) => {
        $crate::trace::event($name, format_args!(""))
    };
    ($name:literal, $($fields:tt)+) => {
        if $crate::trace::enabled() {
            $crate::trace::event($name, format_args!($($fields)+));
        }
    };
}

pub(crate) use trace;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_how_long_ago() {
        let now = now();
        let time = (now as u64 % (1 << 32)) as u32;
        let ago = since(time.wrapping_sub(40));
        assert!((40.0..41.5).contains(&ago), "{ago}");
        // a time a little ahead (rounding) is not taken for one 49 days old
        assert!(since(time.wrapping_add(2)) < 0.0);
    }
}
