//! The Nintendo Switch's performance profiles (switchroot's Linux): the
//! power modes of NVIDIA's `nvpmodel`, which the panel's NVIDIA applet
//! switches between (Console, Handheld, the overclocked ones). Offered on
//! the power button's page, and only on a Switch.
//!
//! A Switch is told by its device tree, not by DMI (an ARM board has no
//! SMBIOS tables for `dmidecode` to read): switchroot's kernel lists a
//! `nintendo,…` board among the compatible ones (`nintendo,nx`, with
//! `nintendo,icosa`, `hoag` or `aula`). The modes are the configuration's
//! `< POWER_MODEL ID=… NAME=… >` sections; the current one is in
//! `nvpmodel`'s status file. Changing it takes root: `pkexec`, which the
//! NVIDIA applet's polkit policy (`com.nvidia.pkexec.nvpmodel`) lets the
//! active session do without a password. The mode stays after a restart,
//! as with the applet.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, PoisonError};

use super::{Msg, output, run};

/// Where the device tree is (the second, where `/proc` has no link to it).
const DEVICE_TREES: [&str; 2] = ["/proc/device-tree", "/sys/firmware/devicetree/base"];

/// The program, as the polkit policy names it.
const NVPMODEL: &str = "/usr/sbin/nvpmodel";

/// The configuration `nvpmodel` reads by default, else the chip's.
const CONF: &str = "/etc/nvpmodel.conf";
const CHIP_CONF: &str = "/etc/nvpmodel/nvpmodel_";

/// What `nvpmodel` writes the current mode to (`pmode:0004 fmode:…`).
const STATUS: &str = "/var/lib/nvpmodel/status";

/// One of the configuration's power modes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mode {
    pub id: u32,
    /// Its name, with spaces for the underscores (`OC CPU`).
    pub name: String,
}

/// The modes, and the one in use; changes are applied in a thread.
pub struct Performance {
    conf: PathBuf,
    modes: Vec<Mode>,
    current: Option<u32>,
    worker: Arc<(Mutex<Work>, Condvar)>,
    sender: calloop::channel::Sender<Msg>,
}

/// What the thread is asked to do.
#[derive(Default)]
struct Work {
    /// The mode to change to, the latest asked for.
    wanted: Option<u32>,
    running: bool,
}

impl Performance {
    /// The modes, on a Switch with `nvpmodel` (and `enabled`); the thread
    /// that changes them tells the menu through `sender` what is in use.
    pub fn find(enabled: bool, sender: calloop::channel::Sender<Msg>) -> Option<Self> {
        if !enabled {
            return None;
        }
        let compatible = DEVICE_TREES
            .iter()
            .find_map(|tree| std::fs::read(Path::new(tree).join("compatible")).ok())?;
        if !is_switch(&compatible) {
            return None;
        }
        if !Path::new(NVPMODEL).is_file() || !super::installed("pkexec") {
            log::info!("a Nintendo Switch without nvpmodel and pkexec: no performance profiles");
            return None;
        }
        let (conf, modes) = configurations(&compatible).into_iter().find_map(|conf| {
            let modes = std::fs::read_to_string(&conf)
                .ok()
                .map(|text| parse_modes(&text))
                .filter(|modes| !modes.is_empty())?;
            Some((conf, modes))
        })?;
        let current = current_mode(&conf);
        log::info!(
            "performance profiles from {}: {} (now {current:?})",
            conf.display(),
            modes.len()
        );
        Some(Self {
            conf,
            modes,
            current,
            worker: Arc::default(),
            sender,
        })
    }

    pub fn modes(&self) -> &[Mode] {
        &self.modes
    }

    pub const fn current(&self) -> Option<u32> {
        self.current
    }

    /// Change to a mode (shown in use at once): false when it already is.
    pub fn choose(&mut self, id: u32) -> bool {
        if self.current == Some(id) {
            return false;
        }
        self.current = Some(id);
        let (lock, _) = &*self.worker;
        let mut work = lock.lock().unwrap_or_else(PoisonError::into_inner);
        work.wanted = Some(id);
        if work.running {
            return true;
        }
        work.running = true;
        drop(work);
        let (worker, conf, sender) = (
            Arc::clone(&self.worker),
            self.conf.clone(),
            self.sender.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("performance".into())
            .spawn(move || apply(&worker, &conf, &sender));
        if let Err(err) = spawned {
            log::warn!("cannot start a thread: {err}");
            lock.lock().unwrap_or_else(PoisonError::into_inner).running = false;
        }
        true
    }

    /// Some modes, as if read from a Switch.
    #[cfg(test)]
    pub fn of(modes: Vec<Mode>, current: Option<u32>) -> Self {
        Self {
            conf: PathBuf::from(CONF),
            modes,
            current,
            worker: Arc::default(),
            sender: calloop::channel::channel().0,
        }
    }

    /// What the thread found in use once done.
    pub const fn settled(&mut self, current: Option<u32>) {
        self.current = current;
    }
}

impl Drop for Performance {
    /// The last change is made before the overlay is gone (`pkexec` wants
    /// the process that asks to be there).
    fn drop(&mut self) {
        let (lock, done) = &*self.worker;
        let mut work = lock.lock().unwrap_or_else(PoisonError::into_inner);
        while work.running {
            work = done.wait(work).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

/// The thread: change to the latest mode asked for until none is left,
/// then tell the menu which one is in use (the old one, when it failed).
fn apply(worker: &(Mutex<Work>, Condvar), conf: &Path, sender: &calloop::channel::Sender<Msg>) {
    let (lock, done) = worker;
    loop {
        let wanted = lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .wanted
            .take();
        let Some(id) = wanted else {
            break;
        };
        let conf = conf.to_string_lossy();
        if !run("pkexec", &[NVPMODEL, "-f", &conf, "-m", &id.to_string()]) {
            log::warn!("the performance profile {id} was not set");
        }
        let current = current_mode(Path::new(conf.as_ref()));
        let mut work = lock.lock().unwrap_or_else(PoisonError::into_inner);
        if work.wanted.is_none() {
            work.running = false;
            done.notify_all();
            drop(work);
            sender.send(Msg::Performance(current)).ok();
            return;
        }
    }
    lock.lock().unwrap_or_else(PoisonError::into_inner).running = false;
    done.notify_all();
}

/// Whether a device tree's compatible list (NUL-separated) is a Switch's.
fn is_switch(compatible: &[u8]) -> bool {
    compatible
        .split(|&byte| byte == 0)
        .any(|entry| entry.starts_with(b"nintendo,"))
}

/// The configurations to look in: the default one, then the chip's
/// (Mariko's `t210b01`, Erista's `t210`).
fn configurations(compatible: &[u8]) -> Vec<PathBuf> {
    let mariko = compatible
        .split(|&byte| byte == 0)
        .any(|entry| entry == b"nvidia,tegra210b01");
    let chip = if mariko { "t210b01" } else { "t210" };
    vec![
        PathBuf::from(CONF),
        PathBuf::from(format!("{CHIP_CONF}{chip}.conf")),
    ]
}

/// The `< POWER_MODEL ID=… NAME=… >` sections, in the file's order.
fn parse_modes(text: &str) -> Vec<Mode> {
    text.lines()
        .filter_map(|line| {
            let inner = line.trim().strip_prefix('<')?.strip_suffix('>')?;
            let mut words = inner.split_whitespace();
            if words.next() != Some("POWER_MODEL") {
                return None;
            }
            let (mut id, mut name) = (None, None);
            for word in words {
                match word.split_once('=') {
                    Some(("ID", value)) => id = value.parse().ok(),
                    Some(("NAME", value)) => name = Some(value.replace('_', " ")),
                    _ => {}
                }
            }
            Some(Mode {
                id: id?,
                name: name.filter(|name| !name.is_empty())?,
            })
        })
        .collect()
}

/// The mode in use: `nvpmodel`'s status file, else what it answers.
fn current_mode(conf: &Path) -> Option<u32> {
    std::fs::read_to_string(STATUS)
        .ok()
        .and_then(|status| parse_status(&status))
        .or_else(|| {
            let conf = conf.to_string_lossy();
            output(NVPMODEL, &["-f", &conf, "-q"]).and_then(|answer| parse_query(&answer))
        })
}

/// `pmode:0004 fmode:quiet` → 4.
fn parse_status(status: &str) -> Option<u32> {
    status
        .split_whitespace()
        .find_map(|word| word.strip_prefix("pmode:"))
        .and_then(|mode| mode.parse().ok())
}

/// `nvpmodel -q`: the mode's name, then its number on a line of its own.
fn parse_query(answer: &str) -> Option<u32> {
    answer
        .lines()
        .rev()
        .find_map(|line| line.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switches_are_told_by_their_device_tree() {
        assert!(is_switch(
            b"nvidia,vali\0nintendo,hoag\0nintendo,nx\0nvidia,tegra210b01\0"
        ));
        assert!(!is_switch(
            b"nvidia,p3449-0000-b00+p3448-0000-b00\0nvidia,jetson-nano\0nvidia,tegra210\0"
        ));
        assert!(!is_switch(b""));
    }

    #[test]
    fn the_chip_picks_its_configuration() {
        let mariko =
            configurations(b"nvidia,fric\0nintendo,aula\0nintendo,nx\0nvidia,tegra210b01\0");
        assert_eq!(
            mariko[1],
            PathBuf::from("/etc/nvpmodel/nvpmodel_t210b01.conf")
        );
        let erista = configurations(b"nvidia,odin\0nintendo,icosa\0nintendo,nx\0nvidia,tegra210\0");
        assert_eq!(erista[0], PathBuf::from(CONF));
        assert_eq!(erista[1], PathBuf::from("/etc/nvpmodel/nvpmodel_t210.conf"));
    }

    #[test]
    fn modes_come_from_the_power_model_sections() {
        let conf = "\
# < POWER_MODEL ID=id_num NAME=mode_name >
< PARAM TYPE=CLOCK NAME=CPU_A57 >
MAX_FREQ /sys/kernel/nvpmodel_emc_cap/cpu
< POWER_MODEL ID=0 NAME=Console >
CPU_A57 MAX_FREQ 1785000
< POWER_MODEL ID=1 NAME=Handheld >
<POWER_MODEL ID=2 NAME=OC_CPU>
< POWER_MODEL NAME=Broken >
< PM_CONFIG DEFAULT=0 >
";
        let modes = parse_modes(conf);
        let names: Vec<(u32, &str)> = modes
            .iter()
            .map(|mode| (mode.id, mode.name.as_str()))
            .collect();
        assert_eq!(names, [(0, "Console"), (1, "Handheld"), (2, "OC CPU")]);
    }

    #[test]
    fn the_current_mode_is_read() {
        assert_eq!(parse_status("pmode:0004 fmode:quiet\n"), Some(4));
        assert_eq!(parse_status("fmode:quiet"), None);
        assert_eq!(
            parse_query("NV Fan Mode:quiet\nNV Power Mode: OC_All\n4\n"),
            Some(4)
        );
        assert_eq!(parse_query("NVPM WARN: power mode is not set!\n"), None);
    }
}
