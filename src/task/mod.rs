//! The task screen: run an installer (or a download) full screen, showing its
//! output, over Retrobox's background.
//!
//! Retrobox starts it (`retrobox-overlay --task ...`) in a process of its
//! own, so none of its memory stays behind while the game runs:
//!
//! * when the emulator a game needs is not installed it asks whether to
//!   install it, runs the installer and shows what it prints (with `--reinstall`
//!   the emulator is there, and it offers to install it again, to update it);
//! * before EmulationStation starts, `--download` runs a download (the default
//!   theme, bezels and shaders) right away and closes when it succeeds;
//! * from the menu's System page, `--update` asks whether to update the
//!   system, then runs the update (`<system settings program> update run`)
//!   and shows what it prints.
//!
//! This module holds what does not need a display: the command line, the
//! steps of the screen and its exit status. The terminal's text is in
//! `terminal`, the installer's process in `process`, the drawing in
//! `ui::task` and the window's events in `app::task_view`.
//!
//! Exit status: 0 installed (launch the game), 10 declined (or download
//! cancelled), 11 install failed, 12 no screen could be opened.

pub mod clipboard;
pub mod process;
pub mod terminal;

use std::path::PathBuf;

/// The lock the task screen holds (one at a time).
pub const LOCK_FILE: &str = "task.lock";

/// Installed, or downloaded.
pub const EXIT_INSTALLED: u8 = 0;
/// The user said no, or cancelled the download.
pub const EXIT_DECLINED: u8 = 10;
/// The installation failed (and the user has seen why).
pub const EXIT_FAILED: u8 = 11;
/// No screen could be opened.
pub const EXIT_UNAVAILABLE: u8 = 12;

/// What the screen is about, from its command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// What is installed or downloaded, as shown to the user.
    pub label: String,
    /// The installer, or the downloader.
    pub command: Vec<String>,
    /// Where the output of the command is saved.
    pub log: PathBuf,
    /// A file that must exist after a successful run.
    pub check: Option<PathBuf>,
    /// The picture behind the screen (a PNG).
    pub background: Option<PathBuf>,
    /// The command downloads the label: run it right away, close when it
    /// succeeds.
    pub download: bool,
    /// The label is installed: offer to install it again (to update it).
    pub reinstall: bool,
    /// The command updates the system (the label is not shown).
    pub update: bool,
    /// The controllers to read (evdev devices); every one if there are none.
    pub pads: Vec<PathBuf>,
    /// The language of the texts (default: from the theme, else the system).
    pub language: Option<String>,
}

impl Options {
    /// Read the arguments that follow `--task`: flags, then `--` and the
    /// command.
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut options = Self {
            label: String::new(),
            command: Vec::new(),
            log: PathBuf::new(),
            check: None,
            background: None,
            download: false,
            reinstall: false,
            update: false,
            pads: Vec::new(),
            language: None,
        };
        let mut arguments = args.iter();
        while let Some(argument) = arguments.next() {
            let mut value = |name: &str| {
                arguments
                    .next()
                    .cloned()
                    .ok_or_else(|| format!("{name} needs a value"))
            };
            match argument.as_str() {
                "--task" => {}
                "--label" => options.label = value("--label")?,
                "--log" => options.log = value("--log")?.into(),
                "--check" => options.check = Some(value("--check")?.into()),
                "--background" => options.background = Some(value("--background")?.into()),
                "--pad" => options.pads.push(value("--pad")?.into()),
                "--lang" => options.language = Some(value("--lang")?),
                "--download" => options.download = true,
                "--reinstall" => options.reinstall = true,
                "--update" => options.update = true,
                "--" => {
                    options.command = arguments.cloned().collect();
                    break;
                }
                other => return Err(format!("unknown argument {other:?}")),
            }
        }
        if options.label.is_empty() {
            return Err("missing --label".into());
        }
        if options.log.as_os_str().is_empty() {
            return Err("missing --log".into());
        }
        if options.command.is_empty() {
            return Err("missing the installer command (after --)".into());
        }
        if [options.download, options.reinstall, options.update]
            .into_iter()
            .filter(|&flag| flag)
            .count()
            > 1
        {
            return Err("--download, --reinstall and --update exclude each other".into());
        }
        Ok(options)
    }
}

/// The steps of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Asking whether to install.
    Ask,
    /// The installer is running.
    Running,
    /// It failed (or could not be run).
    Failed,
    /// It worked.
    Done,
}

/// What a button of the screen does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Yes,
    No,
    Ok,
}

impl Choice {
    /// The id of the button's text.
    pub const fn text_key(self) -> &'static str {
        match self {
            Self::Yes => "task-yes",
            Self::No => "task-no",
            Self::Ok => "task-ok",
        }
    }
}

/// Where the screen is: its step, the focused button and what is asked.
#[derive(Debug, Clone)]
pub struct Model {
    phase: Phase,
    focus: usize,
    download: bool,
    reinstall: bool,
    update: bool,
}

impl Model {
    pub const fn new(options: &Options) -> Self {
        Self {
            phase: Phase::Ask,
            focus: 0,
            download: options.download,
            reinstall: options.reinstall,
            update: options.update,
        }
    }

    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Go to a step: the first button is focused again.
    pub const fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.focus = 0;
    }

    /// The buttons of the step.
    pub const fn buttons(&self) -> &'static [Choice] {
        match self.phase {
            Phase::Ask => &[Choice::Yes, Choice::No],
            Phase::Failed | Phase::Done => &[Choice::Ok],
            Phase::Running => &[],
        }
    }

    pub const fn focus(&self) -> usize {
        self.focus
    }

    pub fn focused(&self) -> Option<Choice> {
        self.buttons().get(self.focus).copied()
    }

    /// Focus the button `index`, if there is one; whether it changed.
    pub fn set_focus(&mut self, index: usize) -> bool {
        let changed = index < self.buttons().len() && index != self.focus;
        if changed {
            self.focus = index;
        }
        changed
    }

    /// Move the focus (round); whether it changed.
    pub fn move_focus(&mut self, delta: isize) -> bool {
        let count = self.buttons().len();
        if count < 2 {
            return false;
        }
        let next = (self.focus as isize + delta).rem_euclid(count as isize) as usize;
        self.set_focus(next)
    }

    /// The id of the text that says what is going on.
    pub const fn message_key(&self) -> &'static str {
        if self.update {
            return match self.phase {
                Phase::Ask => "task-update-ask",
                Phase::Running => "task-updating",
                Phase::Failed => "task-update-failed",
                Phase::Done => "task-updated",
            };
        }
        match (self.download, self.reinstall, self.phase) {
            (true, _, Phase::Ask | Phase::Running) => "task-downloading",
            (true, _, Phase::Failed) => "task-download-failed",
            (false, true, Phase::Ask) => "task-reinstall-ask",
            (false, true, Phase::Running) => "task-reinstalling",
            (false, true, Phase::Failed) => "task-reinstall-failed",
            (false, true, Phase::Done) => "task-reinstalled",
            (false, false, Phase::Ask) => "task-ask",
            (false, false, Phase::Running) => "task-installing",
            (false, false, Phase::Failed) => "task-failed",
            (_, _, Phase::Done) => "task-done",
        }
    }

    /// The exit status when the window is closed, or the screen is stopped
    /// by a signal: whatever was going on is not launched.
    pub const fn quit_status(&self) -> u8 {
        match self.phase {
            Phase::Done => EXIT_INSTALLED,
            Phase::Ask => EXIT_DECLINED,
            Phase::Running | Phase::Failed => EXIT_FAILED,
        }
    }

    /// The exit status of leaving with a button.
    pub const fn status_of(&self, choice: Choice) -> Option<u8> {
        match (choice, self.phase) {
            (Choice::No, Phase::Ask) => Some(EXIT_DECLINED),
            (Choice::Ok, Phase::Done) => Some(EXIT_INSTALLED),
            (Choice::Ok, Phase::Failed) => Some(EXIT_FAILED),
            _ => None,
        }
    }

    /// Back (B, Esc): says no to the question, cancels a download.
    pub const fn back_status(&self) -> Option<u8> {
        match self.phase {
            Phase::Ask => Some(EXIT_DECLINED),
            Phase::Running if self.download => Some(EXIT_DECLINED),
            _ => None,
        }
    }

    /// What a finished command means, given whether the file that proves the
    /// installation is there: the step it leads to, and the line to add to
    /// the terminal when it was not what was expected.
    pub fn finished(&self, status: i32, check: Option<(&std::path::Path, bool)>) -> Outcome {
        match (status, check) {
            (0, Some((path, false))) => Outcome::Failed(Problem::StillMissing(path.to_owned())),
            (0, _) if self.download => Outcome::Leave(EXIT_INSTALLED),
            (0, _) => Outcome::Phase(Phase::Done),
            (status, _) => Outcome::Failed(Problem::Status(status)),
        }
    }
}

/// What went wrong with a command that finished.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// It succeeded, but the file that proves the installation is not there.
    StillMissing(PathBuf),
    /// It failed with this status.
    Status(i32),
}

/// What the end of the command leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Show this step (success, to be confirmed).
    Phase(Phase),
    /// Close with this status, nothing to confirm (a download).
    Leave(u8),
    /// The failure step, saying why.
    Failed(Problem),
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn arguments(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn options(download: bool, reinstall: bool) -> Options {
        Options {
            label: "Eden".into(),
            command: arguments(&["retrobox", "--setup-emulator", "eden"]),
            log: "/tmp/eden.log".into(),
            check: None,
            background: None,
            download,
            reinstall,
            update: false,
            pads: Vec::new(),
            language: None,
        }
    }

    #[test]
    fn the_update_asks_then_says_how_it_went() {
        let parsed = Options::parse(&arguments(&[
            "--task",
            "--update",
            "--label",
            "Retrobox",
            "--log",
            "/tmp/update.log",
            "--",
            "retrobox-system-settings",
            "update",
            "run",
        ]))
        .unwrap();
        assert!(parsed.update);
        let mut model = Model::new(&parsed);
        assert_eq!(model.message_key(), "task-update-ask");
        assert_eq!(model.buttons(), [Choice::Yes, Choice::No]);
        model.set_phase(Phase::Running);
        assert_eq!(model.message_key(), "task-updating");
        model.set_phase(Phase::Failed);
        assert_eq!(model.message_key(), "task-update-failed");
        model.set_phase(Phase::Done);
        assert_eq!(model.message_key(), "task-updated");
        let both = Options::parse(&arguments(&[
            "--update",
            "--reinstall",
            "--label",
            "x",
            "--log",
            "/tmp/x",
            "--",
            "true",
        ]));
        assert!(both.is_err());
    }

    #[test]
    fn reads_the_command_line() {
        let parsed = Options::parse(&arguments(&[
            "--task",
            "--label",
            "RetroArch",
            "--log",
            "/tmp/x.log",
            "--check",
            "/opt/retroarch",
            "--background",
            "/bg.png",
            "--pad",
            "/dev/input/event3",
            "--pad",
            "/dev/input/event4",
            "--lang",
            "es",
            "--reinstall",
            "--",
            "retrobox",
            "--setup-emulator",
            "retroarch",
        ]))
        .unwrap();
        assert_eq!(parsed.label, "RetroArch");
        assert_eq!(parsed.check, Some("/opt/retroarch".into()));
        assert_eq!(parsed.pads.len(), 2);
        assert_eq!(parsed.language.as_deref(), Some("es"));
        assert!(parsed.reinstall && !parsed.download);
        assert_eq!(
            parsed.command,
            ["retrobox", "--setup-emulator", "retroarch"]
        );
    }

    #[test]
    fn the_command_may_hold_flags_of_its_own() {
        let parsed = Options::parse(&arguments(&[
            "--label",
            "X",
            "--log",
            "l",
            "--",
            "tool",
            "--label",
            "--download",
        ]))
        .unwrap();
        assert_eq!(parsed.command, ["tool", "--label", "--download"]);
        assert!(!parsed.download);
    }

    #[test]
    fn rejects_what_is_missing_or_unknown() {
        for bad in [
            &["--log", "l", "--", "x"][..],
            &["--label", "X", "--", "x"][..],
            &["--label", "X", "--log", "l"][..],
            &["--label", "X", "--log", "l", "--bogus", "--", "x"][..],
            &["--label", "X", "--log"][..],
            &[
                "--label",
                "X",
                "--log",
                "l",
                "--download",
                "--reinstall",
                "--",
                "x",
            ][..],
        ] {
            assert!(Options::parse(&arguments(bad)).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn asks_then_leaves_with_the_answer() {
        let mut model = Model::new(&options(false, false));
        assert_eq!(model.buttons(), [Choice::Yes, Choice::No]);
        assert_eq!(model.message_key(), "task-ask");
        assert_eq!(model.focused(), Some(Choice::Yes));
        assert!(model.move_focus(1));
        assert_eq!(model.focused(), Some(Choice::No));
        assert!(model.move_focus(1), "the focus goes round");
        assert_eq!(model.focused(), Some(Choice::Yes));
        assert_eq!(model.status_of(Choice::No), Some(EXIT_DECLINED));
        assert_eq!(model.status_of(Choice::Yes), None);
        assert_eq!(model.back_status(), Some(EXIT_DECLINED));
        assert_eq!(model.quit_status(), EXIT_DECLINED);
    }

    #[test]
    fn running_has_no_buttons_and_only_a_download_is_cancelled() {
        let mut install = Model::new(&options(false, false));
        install.set_phase(Phase::Running);
        assert_eq!(install.buttons(), &[]);
        assert!(!install.move_focus(1));
        assert_eq!(install.back_status(), None);
        assert_eq!(install.quit_status(), EXIT_FAILED);

        let mut download = Model::new(&options(true, false));
        download.set_phase(Phase::Running);
        assert_eq!(download.back_status(), Some(EXIT_DECLINED));
        assert_eq!(download.message_key(), "task-downloading");
    }

    #[test]
    fn the_end_of_the_command_decides_the_next_step() {
        let install = Model::new(&options(false, false));
        assert_eq!(install.finished(0, None), Outcome::Phase(Phase::Done));
        assert_eq!(
            install.finished(0, Some((Path::new("/bin/x"), true))),
            Outcome::Phase(Phase::Done)
        );
        assert_eq!(
            install.finished(0, Some((Path::new("/bin/x"), false))),
            Outcome::Failed(Problem::StillMissing("/bin/x".into()))
        );
        assert_eq!(
            install.finished(2, None),
            Outcome::Failed(Problem::Status(2))
        );
        let download = Model::new(&options(true, false));
        assert_eq!(download.finished(0, None), Outcome::Leave(EXIT_INSTALLED));
    }

    #[test]
    fn a_reinstall_says_so() {
        let mut model = Model::new(&options(false, true));
        assert_eq!(model.message_key(), "task-reinstall-ask");
        model.set_phase(Phase::Failed);
        assert_eq!(model.message_key(), "task-reinstall-failed");
        assert_eq!(model.status_of(Choice::Ok), Some(EXIT_FAILED));
        model.set_phase(Phase::Done);
        assert_eq!(model.message_key(), "task-reinstalled");
        assert_eq!(model.status_of(Choice::Ok), Some(EXIT_INSTALLED));
    }
}
