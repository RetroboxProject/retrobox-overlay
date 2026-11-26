//! The task screen (`--task`): an installer or a download run full screen.
//!
//! The screen is the dim's own surface, covered by one picture (see
//! `ui/task.rs`); the steps and their exits are in `task/mod.rs`. This file
//! is what ties them to the display: the installer's process and its output,
//! the buttons and the keys, the pointer and the controllers.
//!
//! The keyboard reaches the installer while it runs (a password, a yes/no
//! answer), as in a terminal: Enter, Backspace, Tab, the arrows, Ctrl+key and
//! text. Page Up/Down scroll the output, Ctrl+Shift+C copies it (what is
//! selected with the pointer, or all of it) and Ctrl+Shift+A selects it all.
//! A selection made with the pointer is copied when it ends.

use std::time::Duration;

use smithay_client_toolkit::reexports::calloop::channel::{self as loop_channel, Event};
use smithay_client_toolkit::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay_client_toolkit::seat::keyboard::KeyEvent;
use x11rb::protocol::xproto::KeyButMask;
use xkbcommon::xkb;

use super::{BTN_LEFT, BTN_RIGHT, Outcome, PointerInput, PointerKind, STATUS_EVERY, State};
use crate::input::{Input, Nav};
use crate::sound::Sound;
use crate::task::process::{self, Process};
use crate::task::terminal::Terminal;
use crate::task::{Choice, Model, Options, Outcome as Finish, Phase, Problem, clipboard};
use crate::ui::help::Prompt;
use crate::ui::panel::StatusBar;
use crate::ui::task::{Frame, TaskUi};

/// The output is drawn at most this often while it streams in.
const DRAW_EVERY: Duration = Duration::from_millis(33);
/// What a notch of the wheel, or a Page key, scrolls.
const WHEEL_ROWS: usize = 3;

/// Which of Ctrl and Shift are held.
#[derive(Debug, Clone, Copy, Default)]
struct Modifiers {
    ctrl: bool,
    shift: bool,
}

/// How often the cursor blinks (on, then off).
const BLINK: Duration = Duration::from_millis(530);

/// The task screen's state.
pub(super) struct TaskView {
    options: Options,
    model: Model,
    ui: Option<TaskUi>,
    terminal: Terminal,
    process: Option<Process>,
    events: Option<loop_channel::Sender<process::Event>>,
    /// Rows the output is scrolled up from its end (0: following it).
    scroll: usize,
    /// The button under the pointer, and the one a press went down on.
    hover: Option<usize>,
    pressed: Option<usize>,
    /// The pointer is selecting text.
    selecting: bool,
    /// Ctrl and Shift, as last told (Wayland), or from the key events.
    modifiers: Modifiers,
    /// A redraw is on its way (the output streams faster than it is drawn).
    draw_pending: bool,
    /// What types on X11, where the server only says key codes.
    keys: Option<xkb::State>,
    /// The cursor's blink, while its timer runs: whether it shows now.
    blink: Option<bool>,
    /// The status line (the network, the battery, the time), as last read.
    status: StatusBar,
}

impl TaskView {
    pub(super) fn new(options: Options) -> Self {
        Self {
            model: Model::new(&options),
            options,
            ui: None,
            terminal: Terminal::new(),
            process: None,
            events: None,
            scroll: 0,
            hover: None,
            pressed: None,
            selecting: false,
            modifiers: Modifiers::default(),
            draw_pending: false,
            keys: None,
            blink: None,
            status: StatusBar::default(),
        }
    }

    /// The exit status when the screen is stopped from outside.
    pub(super) const fn quit_status(&self) -> u8 {
        self.model.quit_status()
    }

    pub(super) const fn set_modifiers(&mut self, ctrl: bool, shift: bool) {
        self.modifiers = Modifiers { ctrl, shift };
    }
}

/// A key, whichever the display, as far as the screen cares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Key {
    Enter,
    Backspace,
    Tab,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    Delete,
    PageUp,
    PageDown,
    /// Text (a control character with Ctrl held).
    Text(String),
}

impl Key {
    /// From an X keysym (`XK_*`) and the text the key types.
    pub(super) fn from_keysym(keysym: u32, text: &str) -> Option<Self> {
        Some(match keysym {
            0xff0d | 0xff8d => Self::Enter,
            0xff08 => Self::Backspace,
            0xff09 | 0xfe20 => Self::Tab,
            0xff1b => Self::Escape,
            0xff52 | 0xff97 => Self::Up,
            0xff54 | 0xff99 => Self::Down,
            0xff51 | 0xff96 => Self::Left,
            0xff53 | 0xff98 => Self::Right,
            0xff50 | 0xff95 => Self::Home,
            0xff57 | 0xff9c => Self::End,
            0xffff | 0xff9f => Self::Delete,
            0xff55 | 0xff9a => Self::PageUp,
            0xff56 | 0xff9b => Self::PageDown,
            _ if !text.is_empty() => Self::Text(text.to_owned()),
            _ => return None,
        })
    }

    /// What the installer's terminal receives for it.
    fn bytes(&self) -> Vec<u8> {
        match self {
            Self::Enter => b"\r".to_vec(),
            Self::Backspace => vec![0x7f],
            Self::Tab => b"\t".to_vec(),
            Self::Escape => vec![0x1b],
            Self::Up => b"\x1b[A".to_vec(),
            Self::Down => b"\x1b[B".to_vec(),
            Self::Right => b"\x1b[C".to_vec(),
            Self::Left => b"\x1b[D".to_vec(),
            Self::Home => b"\x1b[H".to_vec(),
            Self::End => b"\x1b[F".to_vec(),
            Self::Delete => b"\x1b[3~".to_vec(),
            Self::PageUp | Self::PageDown => Vec::new(),
            Self::Text(text) => text.as_bytes().to_vec(),
        }
    }
}

impl State {
    /// The screen's size is known: build it, and start the download if
    /// that is what was asked.
    pub(super) fn create_task(&mut self) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let Some(ui) = TaskUi::new(
            &self.content.theme,
            &mut self.content.fonts,
            self.dim.size,
            task.options.background.as_deref(),
        ) else {
            log::error!("the screen is too small for the task screen");
            self.outcome = Some(Outcome::Task(crate::task::EXIT_UNAVAILABLE));
            return;
        };
        task.ui = Some(ui);
        if self.display.x11().is_some() {
            task.keys = x11_keys();
        }
        self.schedule_task_status();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let (sender, receiver) = loop_channel::channel::<process::Event>();
        let inserted = self
            .handle
            .insert_source(receiver, |event, &mut (), state| {
                if let Event::Msg(event) = event {
                    state.task_event(event);
                }
            });
        if let Err(err) = inserted {
            log::error!("cannot watch the installer: {}", err.error);
            self.outcome = Some(Outcome::Task(crate::task::EXIT_UNAVAILABLE));
            return;
        }
        task.events = Some(sender);
        if task.options.download {
            self.task_start();
        }
        self.draw_task();
    }

    /// Run the installer.
    fn task_start(&mut self) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let (Some(ui), Some(events)) = (task.ui.as_ref(), task.events.clone()) else {
            return;
        };
        let size = (
            u16::try_from(ui.columns()).unwrap_or(80),
            u16::try_from(ui.visible_rows()).unwrap_or(24),
        );
        task.model.set_phase(Phase::Running);
        task.terminal = Terminal::new();
        task.scroll = 0;
        self.start_blinking();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        match Process::start(&task.options.command, &task.options.log, size, events) {
            Ok(process) => task.process = Some(process),
            Err(err) => {
                log::error!("cannot run {:?}: {err}", task.options.command);
                task.terminal.feed(&format!("{err}\r\n"));
                self.task_failed(None);
            }
        }
    }

    /// The installer printed or ended.
    fn task_event(&mut self, event: process::Event) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        match event {
            process::Event::Output(text) => {
                task.terminal.feed(&text);
                // what is typed or printed shows the cursor at once
                if task.blink.is_some() {
                    task.blink = Some(true);
                }
                self.schedule_task_draw();
            }
            process::Event::Exited(status) => {
                // the thread has ended: this only collects it
                drop(task.process.take());
                let check = task
                    .options
                    .check
                    .as_deref()
                    .map(|path| (path, path.exists()));
                match task.model.finished(status, check) {
                    Finish::Phase(phase) => {
                        task.model.set_phase(phase);
                        self.draw_task();
                    }
                    Finish::Leave(code) => self.outcome = Some(Outcome::Task(code)),
                    Finish::Failed(problem) => self.task_failed(Some(problem)),
                }
            }
        }
    }

    /// The installer failed: say why in the output (and where the log is).
    fn task_failed(&mut self, problem: Option<Problem>) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let texts = &self.content.translator;
        let log = task.options.log.display().to_string();
        let mut lines = vec![String::new()];
        match problem {
            Some(Problem::Status(status)) => lines
                .push(texts.text_strings("task-exit-status", &[("status", &status.to_string())])),
            Some(Problem::StillMissing(path)) => lines.push(texts.text_strings(
                "task-still-missing",
                &[("path", &path.display().to_string())],
            )),
            None => {}
        }
        lines.push(texts.text_strings("task-log", &[("path", &log)]));
        task.terminal.feed(&lines.join("\r\n"));
        task.terminal.feed("\r\n");
        task.scroll = 0;
        task.model.set_phase(Phase::Failed);
        self.draw_task();
    }

    /// Blink the cursor while the command runs: a timer of its own, which
    /// stops once it has ended.
    fn start_blinking(&mut self) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        if task.blink.is_some() {
            return;
        }
        task.blink = Some(true);
        let inserted =
            self.handle
                .insert_source(Timer::from_duration(BLINK), |_, &mut (), state| {
                    let Some(task) = state.task.as_mut() else {
                        return TimeoutAction::Drop;
                    };
                    if task.model.phase() != Phase::Running {
                        task.blink = None;
                        return TimeoutAction::Drop;
                    }
                    task.blink = task.blink.map(|on| !on);
                    state.schedule_task_draw();
                    TimeoutAction::ToDuration(BLINK)
                });
        if inserted.is_err()
            && let Some(task) = self.task.as_mut()
        {
            task.blink = None;
        }
    }

    /// Read the status line now and every second after, drawing the screen
    /// when it changed.
    fn schedule_task_status(&mut self) {
        let bar = self.status_bar();
        if let Some(task) = self.task.as_mut() {
            task.status = bar;
        }
        self.handle
            .insert_source(Timer::from_duration(STATUS_EVERY), |_, &mut (), state| {
                let bar = state.status_bar();
                let Some(task) = state.task.as_mut() else {
                    return TimeoutAction::Drop;
                };
                if task.status != bar {
                    task.status = bar;
                    state.draw_task();
                }
                TimeoutAction::ToDuration(STATUS_EVERY)
            })
            .ok();
    }

    /// Draw soon, once for all the output that arrives meanwhile.
    fn schedule_task_draw(&mut self) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        if task.draw_pending {
            return;
        }
        task.draw_pending = true;
        let inserted =
            self.handle
                .insert_source(Timer::from_duration(DRAW_EVERY), |_, &mut (), state| {
                    if let Some(task) = state.task.as_mut() {
                        task.draw_pending = false;
                    }
                    state.draw_task();
                    TimeoutAction::Drop
                });
        if inserted.is_err() {
            task.draw_pending = false;
            self.draw_task();
        }
    }

    /// The prompts at the bottom, for the step.
    fn task_prompts(&self) -> Vec<Prompt> {
        let Some(task) = self.task.as_ref() else {
            return Vec::new();
        };
        let Some(ui) = task.ui.as_ref() else {
            return Vec::new();
        };
        let (ok, back) = ui.help_icons();
        let texts = &self.content.translator;
        let prompt = |icon: &str, key: &str| Prompt {
            icon: icon.to_owned(),
            label: texts.text(key),
        };
        let mut prompts = Vec::new();
        if task.model.buttons().len() > 1 {
            prompts.push(prompt("left/right", "help-choose"));
        }
        prompts.push(prompt("up/down", "help-scroll"));
        match task.model.phase() {
            Phase::Ask => {
                prompts.push(prompt(&ok, "help-select"));
                prompts.push(prompt(&back, "help-back"));
            }
            Phase::Running if task.model.back_status().is_some() => {
                prompts.push(prompt(&back, "help-cancel"));
            }
            Phase::Running => {}
            Phase::Failed | Phase::Done => prompts.push(prompt(&ok, "help-select")),
        }
        prompts
    }

    /// Draw the screen, or once the compositor has shown the last frame (see
    /// `Late`): the pointer selecting the output moves more often than that.
    pub(super) fn draw_task(&mut self) {
        if self.dim.waits_for_frame() {
            self.late.task = true;
            self.draw_late();
            return;
        }
        self.late.task = false;
        let prompts = self.task_prompts();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let Some(ui) = task.ui.as_mut() else {
            return;
        };
        let texts = &self.content.translator;
        let message =
            texts.text_strings(task.model.message_key(), &[("label", &task.options.label)]);
        let buttons: Vec<String> = task
            .model
            .buttons()
            .iter()
            .map(|choice| texts.text(choice.text_key()))
            .collect();
        let show_output = task.model.phase() != Phase::Ask;
        ui.arrange(&mut self.content.fonts, &message, &buttons, show_output);
        let (_, furthest) = ui.rows(&task.terminal, task.scroll);
        task.scroll = task.scroll.min(furthest);
        let pixmap = ui.render(
            &mut self.content.fonts,
            &Frame {
                buttons: &buttons,
                focus: (!buttons.is_empty()).then(|| task.model.focus()),
                hover: task.hover,
                terminal: show_output.then_some((&task.terminal, task.scroll)),
                cursor: task.model.phase() == Phase::Running && task.blink.unwrap_or(true),
                prompts: &prompts,
                status: &task.status,
            },
        );
        self.dim.set_opacity(&self.display, 1.0);
        self.dim.present(&mut self.display, &pixmap);
        self.dim.commit();
    }

    // ------------------------------------------------------------- choices

    /// What a button does.
    fn task_choose(&mut self, choice: Choice) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        self.sounds.play(Sound::Select);
        match (choice, task.model.phase()) {
            (Choice::Yes, Phase::Ask) => {
                self.task_start();
                self.draw_task();
            }
            _ => {
                if let Some(code) = task.model.status_of(choice) {
                    self.outcome = Some(Outcome::Task(code));
                }
            }
        }
    }

    fn task_activate(&mut self) {
        if let Some(choice) = self.task.as_ref().and_then(|task| task.model.focused()) {
            self.task_choose(choice);
        }
    }

    /// Back: no to the question, cancels a download, closes a result.
    fn task_back(&mut self) {
        let Some(task) = self.task.as_ref() else {
            return;
        };
        let status = task.model.back_status().or_else(|| {
            matches!(task.model.phase(), Phase::Done | Phase::Failed)
                .then(|| task.model.quit_status())
        });
        if let Some(code) = status {
            self.sounds.play(Sound::Back);
            self.outcome = Some(Outcome::Task(code));
        }
    }

    fn task_scroll(&mut self, rows: isize) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let before = task.scroll;
        task.scroll = task.scroll.saturating_add_signed(rows);
        // `draw_task` limits it to what exists
        if task.scroll != before {
            self.draw_task();
        }
    }

    fn task_page(&self) -> isize {
        self.task
            .as_ref()
            .and_then(|task| task.ui.as_ref())
            .map_or(WHEEL_ROWS, |ui| ui.visible_rows().saturating_sub(1).max(1)) as isize
    }

    // --------------------------------------------------------------- input

    /// A press or release on a controller, or on the keyboard's navigation
    /// keys while no installer is waiting for them.
    pub(super) fn task_input(&mut self, input: Input) {
        if !input.pressed {
            return;
        }
        let page = self.task_page();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        match input.nav {
            Nav::Left | Nav::Right
                if task
                    .model
                    .move_focus(isize::from(input.nav == Nav::Right) * 2 - 1) =>
            {
                self.sounds.play(Sound::Scroll);
                self.draw_task();
            }
            Nav::Confirm => self.task_activate(),
            Nav::Back | Nav::Close => self.task_back(),
            Nav::Up => self.task_scroll(1),
            Nav::Down => self.task_scroll(-1),
            Nav::PrevPage | Nav::ZoomOut => self.task_scroll(page),
            Nav::NextPage | Nav::ZoomIn => self.task_scroll(-page),
            _ => {}
        }
    }

    /// A key press (or repeat) from the keyboard.
    pub(super) fn task_key(&mut self, key: &Key) {
        let page = self.task_page();
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let Modifiers { ctrl, shift } = task.modifiers;
        if ctrl
            && shift
            && let Key::Text(text) = key
        {
            match text.to_lowercase().as_str() {
                "c" | "\u{3}" => return self.task_copy(),
                "a" | "\u{1}" => return self.task_select_all(),
                _ => {}
            }
        }
        match key {
            Key::PageUp => return self.task_scroll(page),
            Key::PageDown => return self.task_scroll(-page),
            _ => {}
        }
        if task.model.phase() == Phase::Running {
            if *key == Key::Escape && task.model.back_status().is_some() {
                return self.task_back();
            }
            let bytes = key.bytes();
            if let Some(process) = &task.process
                && !bytes.is_empty()
            {
                process.write(&bytes);
                // typing goes back to the end of the output
                if task.scroll != 0 {
                    task.scroll = 0;
                    self.draw_task();
                }
            }
            return;
        }
        match key {
            Key::Enter | Key::Text(_) if matches!(key, Key::Enter) || is_space(key) => {
                self.task_activate();
            }
            Key::Left => self.task_input(Input::press(Nav::Left)),
            Key::Right | Key::Tab => self.task_input(Input::press(Nav::Right)),
            Key::Up => self.task_scroll(1),
            Key::Down => self.task_scroll(-1),
            Key::Escape | Key::Backspace => self.task_back(),
            _ => {}
        }
    }

    /// Wayland: a key, as it types.
    pub(super) fn wayland_task_key(&mut self, event: &KeyEvent) {
        let text = event.utf8.as_deref().unwrap_or_default();
        if let Some(key) = Key::from_keysym(event.keysym.raw(), text) {
            self.task_key(&key);
        }
    }

    /// X11: a key code (and the modifiers held), as a key.
    pub(super) fn x11_task_key(&mut self, keycode: u8, mask: KeyButMask, pressed: bool) {
        let Some(task) = self.task.as_mut() else {
            return;
        };
        task.modifiers = Modifiers {
            ctrl: mask.contains(KeyButMask::CONTROL),
            shift: mask.contains(KeyButMask::SHIFT),
        };
        let Some(keys) = task.keys.as_mut() else {
            return;
        };
        let code = xkb::Keycode::new(u32::from(keycode));
        let key = pressed
            .then(|| {
                let text = keys.key_get_utf8(code);
                Key::from_keysym(keys.key_get_one_sym(code).raw(), &text)
            })
            .flatten();
        keys.update_key(
            code,
            if pressed {
                xkb::KeyDirection::Down
            } else {
                xkb::KeyDirection::Up
            },
        );
        if let Some(key) = key {
            self.task_key(&key);
        }
    }

    // ------------------------------------------------------------- pointer

    pub(super) fn task_pointer(&mut self, event: &PointerInput) {
        let point = (event.position.0 as f32, event.position.1 as f32);
        let Some(task) = self.task.as_mut() else {
            return;
        };
        let Some(ui) = task.ui.as_ref() else {
            return;
        };
        let button = ui.button_at(point);
        match event.kind {
            PointerKind::Enter | PointerKind::Motion => {
                if task.selecting {
                    let scroll = task.scroll;
                    let end = ui.position_at(&task.terminal, scroll, point);
                    if end.is_some() {
                        task.terminal.selection_end = end;
                        self.draw_task();
                    }
                } else if task.hover != button {
                    task.hover = button;
                    self.draw_task();
                }
            }
            PointerKind::Leave => {
                if task.hover.take().is_some() {
                    self.draw_task();
                }
            }
            PointerKind::Press(BTN_LEFT) => {
                if button.is_some() {
                    task.pressed = button;
                } else if ui.in_terminal(point) {
                    let scroll = task.scroll;
                    let at = ui.position_at(&task.terminal, scroll, point);
                    task.terminal.anchor = at;
                    task.terminal.selection_end = at;
                    task.selecting = at.is_some();
                    self.draw_task();
                }
            }
            PointerKind::Release(BTN_LEFT) => {
                let pressed = task.pressed.take();
                if task.selecting {
                    task.selecting = false;
                    if task.terminal.selection().is_some() {
                        self.task_copy();
                    } else {
                        task.terminal.clear_selection();
                    }
                    self.draw_task();
                } else if let (Some(pressed), Some(choice)) = (
                    pressed,
                    pressed.and_then(|index| task.model.buttons().get(index).copied()),
                ) && button == Some(pressed)
                {
                    self.task_choose(choice);
                }
            }
            PointerKind::Press(BTN_RIGHT) => self.task_back(),
            PointerKind::Wheel(steps) if steps != 0 => {
                self.task_scroll(-(steps as isize) * WHEEL_ROWS as isize);
            }
            _ => {}
        }
    }

    // ----------------------------------------------------------- clipboard

    /// Copy the selection, or all the output when nothing is selected.
    fn task_copy(&mut self) {
        let Some(task) = self.task.as_ref() else {
            return;
        };
        let text = task.terminal.selected_text();
        if !text.is_empty() {
            clipboard::copy(text, self.display.is_wayland());
        }
    }

    fn task_select_all(&mut self) {
        if let Some(task) = self.task.as_mut() {
            task.terminal.select_all();
            self.draw_task();
        }
    }

    /// The task screen is the only thing on screen: another window taking
    /// the keyboard must not stop the installation.
    pub(super) fn task_is_open(&self) -> bool {
        self.task.is_some()
    }
}

fn is_space(key: &Key) -> bool {
    matches!(key, Key::Text(text) if text == " ")
}

/// The keymap of the session (the environment's layout), as a state to ask
/// what each key code types.
fn x11_keys() -> Option<xkb::State> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap =
        xkb::Keymap::new_from_names(&context, "", "", "", "", None, xkb::KEYMAP_COMPILE_NO_FLAGS)?;
    Some(xkb::State::new(&keymap))
}
