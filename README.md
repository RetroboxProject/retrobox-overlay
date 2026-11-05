# retrobox-overlay

The in-game menu of [Retrobox](https://github.com/RetroboxProject/retrobox-core). Press the
controller's hotkey alone (Home/Guide), Hotkey+A, or Shift+F7 while playing: a side menu slides in
over the dimmed game, drawn with the menu style of the active EmulationStation theme, with a
clock and the date on the other side.

It is a small Rust program (a few MB, ~35 ms from start to the menu on screen) that runs only
while it is shown: nothing stays resident during the game.

## How it fits in Retrobox

* **Opening and closing.** gamepadly (Retrobox's pad-to-keyboard mapper) runs `retrobox-overlay`
  on the standard hotkeys, which retrobox-core only binds when `retrobox-overlay` is installed.
  Started again while it is open (the hotkey, Shift+F7), it closes the open one: a single
  instance holds `overlay.lock` in Retrobox's runtime directory and gets `SIGUSR1`.
* **The game.** The launcher publishes the running game in `session.json` (runtime directory):
  its controllers, how the emulator performs each standard action, and the paths below. Without
  a running game the overlay does nothing (`--demo` shows it anyway, to try a theme).
* **The theme.** On every game launch, Retrobox's EmulationStation build writes its resolved menu
  theme (`es-menu-theme.json`: colours, fonts and their pixel sizes, background...) after the
  theme's variables, subsets and the user's theme choices are applied
  (patch 0006 of `retrobox-emulationstation`). The overlay draws with it, scaled to the screen;
  without it, it looks like EmulationStation's default menus.
* **The game information** comes from `/tmp/game.xml`, which EmulationStation exports at launch
  (name, developer, release date...).
* **Controllers.** While open, the overlay grabs the game's controllers (evdev), so neither the
  emulator nor gamepadly sees a press. It reads them with the Linux gamepad conventions
  (`BTN_SOUTH`/`BTN_EAST`, d-pad, left stick, `BTN_MODE`), following EmulationStation's
  *Switch confirm & cancel buttons*. Home or B closes it; the keyboard works too (arrows, Enter,
  Esc, F7), and the mouse: hovering highlights an entry, a click opens it, the wheel moves the
  selection, and a click outside the menu or a right click closes it.
* **The emulator.** `session.json` says how the running emulator performs each standard hotkey
  action (a key chord, a command, or the emulator's own controller combination with its
  keyboard shortcut). From that the menu offers what the emulator can do: save and load
  state, the disc tray and next/previous disc (`.m3u` playlists), reset, the emulator's menu
  and a screenshot. Chords are
  pressed on a virtual keyboard of the overlay's own (uinput: the user needs write access to
  `/dev/uinput`, as for gamepadly). The game is paused (its pause toggle) while the menu is
  open and resumed when it closes; emulators without a pause key, such as PC games, keep
  running.
* **Save states** (RetroArch). *Save state* and *Load state* open a page of four slots, each with
  the preview RetroArch saves with the state and its date. The slots are RetroArch's own
  `<saves>/<folder>/<game>.state1`…`state4` files, so EmulationStation lists the same states.
  The overlay talks to RetroArch through the command pipe the launcher gives it as standard
  input (`stdin_cmd_enable`): `LOAD_STATE_SLOT N` to load; to save, it moves RetroArch's current
  slot to the chosen one (`STATE_SLOT_PLUS`/`MINUS`, keeping track of it, since Retrobox leaves
  no other way to change it) and sends `SAVE_STATE`.
* **Leaving the game** runs `retrobox-emukill`.

## Requirements

* A Wayland compositor with `wlr-layer-shell`: KWin (Plasma), labwc, sway and other wlroots
  compositors. GNOME has no layer-shell and is not supported.
* `wp_alpha_modifier` (KWin 6.1+, labwc 0.9+) adds the fade-in; without it the menu only slides.
* retrobox-core and retrobox-emulationstation (the packages depend on both).

## Settings

`overlay.ini` in `/etc/retrobox` and `~/.config/retrobox` (the user's file wins, key by key).
See [`data/overlay.ini`](data/overlay.ini) for every key: the menu's side, the dim opacity, the
animation length, whether to pause the game, and the clock (on/off, position, colour, `strftime` formats). Day and month
names follow the language, which is EmulationStation's (else the system's): English, Spanish,
Catalan, Galician, Basque, Portuguese, French, Italian, German, Dutch, Polish and Russian.

## Building

```bash
cargo build --release          # target/release/retrobox-overlay
cargo test && cargo clippy     # what CI checks (clippy pedantic, warnings are errors)
cargo deb --no-build           # .deb  (cargo install cargo-deb)
cargo generate-rpm             # .rpm  (cargo install cargo-generate-rpm)
```

It needs `libxkbcommon` (development files to build). GitHub Actions build the `.deb` and
`.rpm` for amd64 and arm64 on every `v*` tag and publish them with a `SHA256SUMS` file, like the
other Retrobox components.

Run it by hand with `RUST_LOG=debug retrobox-overlay --demo` from a Wayland session.

## License

GPL-2.0-only, like Retrobox.
