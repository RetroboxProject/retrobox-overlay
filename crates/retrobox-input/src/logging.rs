//! The log of a process: stderr from a terminal, else a file of its own in
//! Retrobox's state directory (no one reads the output of a process started
//! by another).

use std::io::IsTerminal as _;

use crate::paths;

/// Log to stderr from a terminal, else to the log file `name`; the file, if
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
    if !std::io::stderr().is_terminal() {
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
