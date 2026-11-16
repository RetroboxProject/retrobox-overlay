//! The log of a process: stderr from a terminal or to systemd's journal,
//! else a file of its own in Retrobox's state directory (no one reads the
//! output of a process started by another).

use std::io::IsTerminal as _;
use std::os::fd::AsFd as _;

use crate::paths;

/// Whether stderr goes to systemd's journal: `JOURNAL_STREAM` names its
/// device and inode (a child that inherited the variable but not the stream
/// does not count).
fn stderr_is_journal() -> bool {
    let Ok(stream) = std::env::var("JOURNAL_STREAM") else {
        return false;
    };
    let Some((device, inode)) = stream.split_once(':') else {
        return false;
    };
    nix::sys::stat::fstat(std::io::stderr().as_fd())
        .is_ok_and(|stat| device.parse() == Ok(stat.st_dev) && inode.parse() == Ok(stat.st_ino))
}

/// Log to stderr from a terminal or to the journal, else to the log file
/// `name`; the file, if
/// it is the log (the caller may empty it once it knows it is the one that
/// writes there).
pub fn init(name: &str) -> Option<std::fs::File> {
    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    // every line says how long after the start it was written
    let start = std::time::Instant::now();
    builder.format(move |buf, record| {
        use std::io::Write;
        writeln!(
            buf,
            "[{:>5} ms {:5} {}] {}",
            start.elapsed().as_millis(),
            record.level(),
            record.target(),
            record.args()
        )
    });
    if !std::io::stderr().is_terminal() && !stderr_is_journal() {
        let path = paths::log_file(name);
        let file = path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
            });
        if let Ok(file) = file
            && let Ok(target) = file.try_clone()
        {
            builder.target(env_logger::Target::Pipe(Box::new(target)));
            builder.init();
            return Some(file);
        }
    }
    builder.init();
    None
}
