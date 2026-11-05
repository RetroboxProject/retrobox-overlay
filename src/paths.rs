//! Where Retrobox keeps things, resolved the same way as `retrobox.paths`.
//!
//! The overlay is started from a game launch (gamepadly runs it), so the
//! `RETROBOX_*` variables the launcher exports are normally there; the XDG
//! fallbacks only matter when it is started by hand.

use std::env;
use std::path::PathBuf;

use nix::unistd::getuid;

fn env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Retrobox's private runtime directory (session.json, the menu theme...).
pub fn runtime_dir() -> PathBuf {
    if let Some(dir) = env_path("RETROBOX_RUNTIME_DIR") {
        return dir;
    }
    if let Some(xdg) = env_path("XDG_RUNTIME_DIR").filter(|dir| dir.is_absolute()) {
        return xdg.join("retrobox");
    }
    env::temp_dir().join(format!("retrobox-{}", getuid()))
}

/// The configuration directories, lowest priority first: system, then user.
pub fn config_dirs() -> Vec<PathBuf> {
    let system = env_path("RETROBOX_SYSCONFDIR").unwrap_or_else(|| PathBuf::from("/etc/retrobox"));
    let user = env_path("RETROBOX_CONFIGDIR").unwrap_or_else(|| {
        env_path("XDG_CONFIG_HOME")
            .or_else(|| env_path("HOME").map(|home| home.join(".config")))
            .unwrap_or_else(|| PathBuf::from("."))
            .join("retrobox")
    });
    vec![system, user]
}

/// The overlay's log when it is not run from a terminal (gamepadly starts it
/// with no one reading its output): `<state dir>/logs/overlay.log`.
pub fn log_file() -> PathBuf {
    env_path("RETROBOX_STATEDIR")
        .unwrap_or_else(|| {
            env_path("XDG_STATE_HOME")
                .or_else(|| env_path("HOME").map(|home| home.join(".local").join("state")))
                .unwrap_or_else(env::temp_dir)
                .join("retrobox")
        })
        .join("logs")
        .join("overlay.log")
}

/// A Retrobox command: from the install's `bin/` when known, else from `$PATH`.
pub fn retrobox_command(name: &str) -> PathBuf {
    env_path("RETROBOX_ROOTDIR")
        .map(|root| root.join("bin").join(name))
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from(name))
}
