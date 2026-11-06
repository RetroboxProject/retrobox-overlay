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
  (`BTN_SOUTH`/`BTN_EAST`, d-pad, left stick, shoulders, `BTN_MODE`), following EmulationStation's
  *Switch confirm & cancel buttons*. Home or B closes it; the keyboard works too (arrows, Enter,
  Esc, F7), and the mouse: hovering highlights an entry, a click opens it, the wheel moves the
  selection, and a click outside the menu or a right click closes it.
* **The emulator.** The overlay only asks; the launcher does. While the game runs, the launcher
  serves a control channel (a Unix socket, `control` in `session.json`) and carries out each
  request the best way the emulator allows: RetroArch's command pipe (its standard input,
  `stdin_cmd_enable`), else a key chord on a virtual keyboard the launcher creates when the game
  starts, else a command line. The menu offers what the channel says the emulator can do:
  save and load state, the disc tray and next/previous disc (`.m3u` playlists), reset, the
  emulator's menu and a screenshot, each with its icon.
* **Discs** (`.m3u` playlists). A line under the game's name shows the disc in the drive and
  whether the tray is open. The tray entry opens or closes it, and next/previous disc change
  the disc, like Batocera: only with the tray open, and not past the first or last disc (the
  entries are dimmed otherwise). These are done with the menu open, which shows the result.
  The launcher keeps track of the disc (starting from the one RetroArch restores) because every
  disc change goes through its control channel, the disc hotkeys too (`retrobox-control`).
* **Pause.** The game is paused when the overlay opens (asked before it shows itself, while the
  game still has the keyboard focus) and runs again only when you go back to it: *Continue*
  or an action. Closing the overlay any other way (back, Home, a click outside, another window
  taking the focus) leaves it paused. Pausing and resuming are absolute, not toggles: with
  RetroArch the launcher asks it first (`GET_STATUS`), so *Continue* always resumes the game,
  whoever paused it. Emulators without a pause, such as PC games, keep running.
* **The desktop stays usable.** The menu takes the keyboard on demand, not exclusively: when the
  compositor gives it to another window (Alt+Tab, a click elsewhere), the overlay closes.
* **Save states** (RetroArch). *Save state* and *Load state* open a page of ten slots (it
  scrolls when they do not all fit), each with the preview RetroArch saves with the state and
  its date. The slots are RetroArch's own `<saves>/<system>/<game>.state1`…`state10` files,
  where EmulationStation looks too.
  Loading is `LOAD_STATE_SLOT N`; for saving the launcher moves RetroArch's current slot to the
  chosen one (`STATE_SLOT_PLUS`/`MINUS`, keeping track of it, since Retrobox leaves no other way
  to change it) and sends `SAVE_STATE`.
* **The manual.** When the game has a scraped manual (`manual` in `game.xml`, a PDF), the menu
  offers *Instruction manual*, a viewer that works like EmulationStation's: the whole page,
  Left/Right (or a click on either half) to turn pages, Confirm to zoom to the screen's width;
  zoomed, L/R zoom out and in while held, the d-pad or stick moves the page, and Back returns
  to the whole page. The triggers and Page Up/Down turn pages at any zoom; the mouse wheel zooms
  and the left button drags. Back from the whole page returns to the menu, where the manual
  stays at the page it was left on.
  Pages are rendered by poppler's `pdftoppm` in a thread, as raw PPM (no PNG to compress and
  decode), the page on screen first and the pages either side ahead, and kept (up to 128 MB).
  A page is uploaded once, to a subsurface: the compositor crops and scales it with
  `wp_viewporter`, so zooming and moving draw nothing, and the page is rendered sharp once a
  zoom settles. `scaling = overlay` in `[manual]` makes the overlay crop and scale instead, for
  compositors that show scaled buffers wrong (wlroots 0.17's software renderer blanks the
  screen).
* **Help prompts.** Like EmulationStation, the menu and the manual show at the bottom of the
  screen what the buttons do, in the theme's help style (`helpsystem` of its system view:
  position, colours, font and icons, the theme's own icons included), which ES exports with
  the menu theme. Without it they look like ES's defaults, with the icons of Retrobox's ES.
* **Leaving the game** runs `retrobox-emukill`.

## Requirements

* A Wayland compositor with `wlr-layer-shell`: KWin (Plasma), labwc, sway and other wlroots
  compositors. GNOME has no layer-shell and is not supported.
* `wp_alpha_modifier` (KWin 6.1+, labwc 0.9+) adds the fade-in; without it the menu only slides.
* retrobox-core and retrobox-emulationstation (the packages depend on both).
* poppler's command line tools (`poppler-utils`, recommended by the packages) for the manuals;
  without them the menu has no manual entry.

## Settings

`overlay.ini` in `/etc/retrobox` and `~/.config/retrobox` (the user's file wins, key by key).
See [`data/overlay.ini`](data/overlay.ini) for every key: the menu's side, the dim opacity, the
animation length, whether to pause the game, the clock (on/off, position, colour, `strftime`
formats) and who scales the manual. Day and month
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

The menu icons are [Lucide](https://lucide.dev)'s, under the ISC license
([`assets/icons/LICENSE`](assets/icons/LICENSE)).
