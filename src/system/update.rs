//! Updating the system, from the System page, done in the task screen, which
//! shows what the updater prints as it goes.
//!
//! On a Nintendo Switch with nx-debian's `nxd-update`, that is the updater
//! (Debian, L4T and the Switch's own packages, and Retrobox's through
//! retrobox-update on its Retrobox image), run as root with `pkexec`
//! (nx-debian's polkit action for it). Elsewhere it is offered when the
//! machine's system settings program (retrobox-system-settings) says it can
//! (`<program> update available`), and `<program> update run` does it
//! (retrobox-update as root). A machine with neither is an ordinary
//! computer, updated its own way: nothing is offered.
//!
//! The program is the one EmulationStation's console mode uses: `[core]
//! system_settings` of retrobox.ini (`/etc/retrobox` and
//! `~/.config/retrobox`, with their `retrobox.ini.d`), else
//! `retrobox-system-settings` in the `PATH`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use ini::Ini;
use retrobox_input::paths;

/// The task screen's log of the last update.
const LOG: &str = "update.log";

/// nx-debian's updater, where its polkit action expects it.
const NXD_UPDATE: &str = "/usr/bin/nxd-update";

/// What runs the update: the updater's command line.
fn updater() -> Option<Vec<String>> {
    if super::performance::on_switch() && Path::new(NXD_UPDATE).is_file() {
        if !super::installed("pkexec") {
            log::info!("a Nintendo Switch without pkexec: no system update");
            return None;
        }
        return Some(
            ["pkexec", "--disable-internal-agent", NXD_UPDATE]
                .map(String::from)
                .to_vec(),
        );
    }
    let program = program()?;
    let can = Command::new(&program)
        .args(["update", "available"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !can {
        log::info!("{}: no system update here", program.display());
        return None;
    }
    Some(vec![
        program.to_string_lossy().into_owned(),
        "update".into(),
        "run".into(),
    ])
}

/// The system settings program, if there is one.
fn program() -> Option<PathBuf> {
    let configured = ini_files(&paths::config_dirs())
        .iter()
        .filter_map(|file| Ini::load_from_file(file).ok())
        .filter_map(|ini| {
            ini.get_from(Some("core"), "system_settings")
                .map(|value| value.trim().trim_matches('"').to_owned())
        })
        .rfind(|value| !value.is_empty());
    let program = configured.map_or_else(
        || in_path("retrobox-system-settings"),
        |value| Some(PathBuf::from(value)),
    )?;
    program.is_file().then_some(program)
}

/// The retrobox.ini files of these configuration directories, lowest
/// priority first: each one's retrobox.ini, then its drop-ins by name.
fn ini_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        files.push(dir.join("retrobox.ini"));
        let mut dropins: Vec<PathBuf> = std::fs::read_dir(dir.join("retrobox.ini.d"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "ini"))
            .collect();
        dropins.sort();
        files.extend(dropins);
    }
    files.retain(|file| file.is_file());
    files
}

fn in_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(name))
            .find(|candidate| candidate.is_file())
    })
}

/// Whether the system can be updated from here.
pub fn available() -> bool {
    updater().is_some()
}

/// The task screen's command line for the update with `updater`.
fn task_command(overlay: &Path, updater: &[String], log: &Path) -> Command {
    let mut command = Command::new(overlay);
    command
        .args(["--task", "--update", "--label", "Retrobox", "--log"])
        .arg(log)
        .arg("--")
        .args(updater);
    command
}

/// Open the task screen with the update (once the menu is gone): a process
/// of its own, which outlives this one.
pub fn start() {
    let Some(updater) = updater() else {
        log::warn!("no system updater here: nothing to update with");
        return;
    };
    let log = paths::log_file(LOG);
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir).ok();
    }
    let mut command = task_command(&paths::sibling_command("retrobox-overlay"), &updater, &log);
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    match command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => log::info!("the update's task screen started (pid {})", child.id()),
        Err(err) => log::warn!("cannot start the update's task screen: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_task_screen_runs_the_program_update() {
        let updater = [
            "/opt/retrobox/bin/retrobox-system-settings",
            "update",
            "run",
        ]
        .map(String::from);
        let command = task_command(
            Path::new("/usr/bin/retrobox-overlay"),
            &updater,
            Path::new("/tmp/update.log"),
        );
        let args: Vec<_> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "--task",
                "--update",
                "--label",
                "Retrobox",
                "--log",
                "/tmp/update.log",
                "--",
                "/opt/retrobox/bin/retrobox-system-settings",
                "update",
                "run"
            ]
        );
    }

    #[test]
    fn the_ini_files_go_system_then_user_with_their_dropins() {
        let dir = std::env::temp_dir().join(format!("overlay-update-{}", std::process::id()));
        let system = dir.join("etc");
        let user = dir.join("home");
        for path in [
            system.join("retrobox.ini"),
            system.join("retrobox.ini.d/50-b.ini"),
            system.join("retrobox.ini.d/10-a.ini"),
            system.join("retrobox.ini.d/notes.txt"),
            user.join("retrobox.ini.d/60-c.ini"),
        ] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, "[core]\n").unwrap();
        }
        let files = ini_files(&[system.clone(), user.clone()]);
        assert_eq!(
            files,
            [
                system.join("retrobox.ini"),
                system.join("retrobox.ini.d/10-a.ini"),
                system.join("retrobox.ini.d/50-b.ini"),
                user.join("retrobox.ini.d/60-c.ini"),
            ]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
