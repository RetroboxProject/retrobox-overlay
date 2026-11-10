# retrobox-overlay

The in-game menu of [Retrobox](https://github.com/RetroboxProject/retrobox-core). Hold the
controller's hotkey (Home/Guide; Select+A on controllers without one) for a moment, or press
Shift+F7, while playing: a
side menu slides in over the dimmed game, drawn with the menu style of the active
EmulationStation theme, with a clock and the date in the top corner of the other side.

It is a small Rust program (a few MB, ~35 ms from start to the menu on screen). What stays
resident is a small daemon that only reads the controllers and keyboards; the menu itself runs
only while it is shown.

## How it fits in Retrobox

* **Opening and closing.** Retrobox starts `retrobox-overlay --daemon` with EmulationStation.
  It reads every controller and keyboard (evdev, without grabbing them: EmulationStation and
  the games get every input as before) and opens the overlay (`retrobox-overlay --show`, a
  process of its own) when a controller's dedicated hotkey (Home/Guide) is held alone for
  `hold_ms` (750 ms), when Select and A (Nintendo's A: the right face button) are held
  together as long on a controller without one (its hotkey is Select, as in
  EmulationStation, and Select alone belongs to the game), or on Shift+F7. Any other button, the d-pad or a stick
  pushed meanwhile cancels the hold until everything is released, so the hotkey's own
  combinations never open it. A hold is used rather than a press because the hotkey is also
  the modifier of every Hotkey+… combination: a press alone could not be told apart from the
  start of one. `retrobox-overlay` started on its own asks the daemon to open or close the
  overlay (`SIGUSR1`); without a daemon it shows the overlay itself, as before. Only one
  overlay runs at a time (`overlay.lock` in Retrobox's runtime directory); a second start
  closes it. The `overlay` setting of retrobox-core (global, per system or per game) turns it
  off over a game.
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
  load and save state, the disc tray and next/previous disc, reset, the emulator's menu
  (RetroArch's only, for now) and a screenshot, each with its icon.
* **Discs.** Games of a platform with an optical drive (`disc_drive: true` in the system's
  YAML in retrobox-core: Mega-CD, Saturn, PlayStation, Dreamcast...) always have the tray entry,
  which also reaches a console's BIOS menu with the tray open. A game of several discs (an
  `.m3u` playlist) also has next/previous disc, and a line under the game's name shows the
  disc in the drive; a line tells whether the tray is open. Next/previous change the disc, like
  Batocera: only with the tray open, and not past the first or last disc (the entries are
  dimmed otherwise). These are done with the menu open, which shows the result.
  The launcher keeps track of the disc (starting from the one RetroArch restores) because every
  disc change goes through its control channel, the disc hotkeys too (`retrobox-control`).
* **Light on the game underneath.** The game keeps running, so the overlay costs the compositor
  as little as it can: the dim is a single pixel the compositor stretches over the screen
  (`wp_single_pixel_buffer` and `wp_viewporter`, which KWin 6 and wlroots have), a redraw only
  hands over the rows that changed (a slider's row, the clock's digits) and nothing when none
  did, the help prompts are drawn again only when they change, and a slider's changes reach
  the system at most every 50 ms. The daemon reads the controllers into one reused buffer and
  only looks at the sticks while a hold is under way. Some cost stays: while any surface is
  over a fullscreen game, the compositor cannot hand the game's frames straight to the screen
  and composites them, as with Steam's overlay.
* **No pause.** Like Steam's overlay, the overlay does not pause the game: most console games
  have a pause of their own, and RetroArch's menu (*Emulator menu*) pauses the rest.
* **No window animations on top.** Under KDE Plasma the surfaces use the `on-screen-display`
  namespace, which KWin's open and close effects (Scale, Fade, Glide) leave alone, so only the
  overlay's own slide and fade play; elsewhere it is `retrobox-overlay`. On X11 the windows are
  override-redirect and typed as on-screen displays.
* **X11.** The same menu, in override-redirect windows with a 32-bit visual on the primary
  monitor (RandR). The overlay grabs the keyboard, so a window manager handing the focus back to
  the game (xfwm4 does) cannot take the menu's keys away, and it raises itself again whenever
  another window is restacked above it. Closing gives the keyboard focus back to the game,
  through the window manager too (xfwm4 would leave it nowhere). The manual's page is cropped and scaled by the overlay (no `wp_viewporter`).
* **The game keeps the focus.** Opened from a controller, the overlay does not take the
  keyboard (`take_keyboard = auto`): the game stays the focused window, because many games and
  emulators slow down, or lower their frame rate, when they lose it. The controller is all the
  menu needs; the mouse and touch work too. Opened from the keyboard (Shift+F7) it takes the
  keyboard, on demand and not exclusively: when the compositor gives it to another window
  (Alt+Tab, a click elsewhere), the overlay goes away, like Steam's (on X11, when the focus
  moves to another window). `take_keyboard = yes` always takes it, `no` never does.
* **Save states** (RetroArch). *Load state* and *Save state* open a page of ten slots (it
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
* **Outside games** (in EmulationStation) the same hold opens the system's menu, like the
  Switch's quick settings: *Sleep mode* (`systemctl suspend`; `standby` in `[system]`), and the
  brightness and volume sliders, which games get too, under the game's entries.
* **Sliders.** Left/Right (d-pad, stick, arrow keys) move them by 5 %, holding repeats; the
  mouse moves them with a click or a drag on the track. Changes apply at once, from a thread
  that only applies the latest value (a held direction asks for many), and the last one is
  applied before the overlay exits. The sliders are there when the menu shows: the volume is
  read in a thread from the moment the overlay starts (the menu waits for it 60 ms at most),
  and finding the brightness's screen starts no program at all. A slider is left out when
  there is nothing to control.
  * **Brightness** of the primary screen: the only screen that is on (`/sys/class/drm`), else
    the output the overlay shows on (Wayland: the compositor puts it on the screen in use), or
    X11's primary output (RandR, outside Wayland: XWayland's outputs have names of their own).
    A built-in panel (eDP, LVDS, DSI) uses the kernel's backlight, written in sysfs when
    allowed, else through logind (`busctl`, allowed to the active session without root), else
    `brightnessctl`; the first way that works is kept, and it never goes below 5 % so the
    screen does not go black. Each change costs the driver some work (NVIDIA's `nvidia_0`
    stalls the display a little, and refuses a change while it is still fading to the last
    one, fading to each value on its own), so the backlight changes once, when the slider has
    stopped for 250 ms, never on the way; the same value is never written twice, and a
    refused change is tried again a moment later (with the latest value) rather than another
    way. An external monitor uses DDC/CI through `ddcutil` on its I2C bus
    (`--bus`, so a change does not detect the monitors again): the daemon looks for the
    monitors that answer when it starts and leaves them in `overlay-ddc.json` in Retrobox's
    runtime directory, with their brightness, which the slider shows at once. Each DDC/CI
    exchange goes through the GPU's driver (NVIDIA's stalls the display a little meanwhile),
    so the monitor is never read when the overlay opens (the cache keeps the value last read or
    set; one changed with the monitor's own buttons shows from the next Retrobox start) and it
    is written once per slide, like a backlight.
  * **Volume** of the default output: PipeWire (`wpctl`), else PulseAudio (`pactl`, which also
    works with PipeWire's PulseAudio server). At 0 the output is muted; above, unmuted.
* **Touch screens and dragging.** A finger is the left button: a tap opens an entry (or sets a
  slider where it lands on its track), a drag up or down scrolls the entries, a drag along a
  slider moves it; the mouse drags the same way. Entries open on release, so a drag never opens
  one. `row_scale` in `[touch]` makes the rows taller than EmulationStation's (the text stays
  the same size) so they are easier to hit: 1.5 to 1.66 suits small touch screens.
* **Status line**, above the menu's header: the network (a cable wins over Wi-Fi, whose icon
  shows the signal), the battery (the computer's own: a controller's is left out) with its
  percentage (`battery_percent` in `[status]`), and the time when the clock is the small one
  (`size = small` in `[clock]`, with its own format, `small_format`); the big clock keeps its
  corner. They are read again every second, from sysfs.

## Requirements

* A Wayland compositor with `wlr-layer-shell` (KWin, labwc, sway and other wlroots compositors),
  or an X11 server: Plasma on X11, for instance where the GPU driver has no good Wayland support
  (Nvidia Tegra, as on Switchroot). Without layer-shell (GNOME) it falls back to X11 through
  XWayland.
* On Wayland, `wp_alpha_modifier` (KWin 6.1+, labwc 0.9+) adds the fade-in; without it the menu
  only slides. On X11 a compositing manager (KWin's, picom...) makes the dim translucent and
  fades the menu in (`_NET_WM_WINDOW_OPACITY`); without one the dim is opaque black.
* retrobox-core and retrobox-emulationstation (the packages depend on both).
* poppler's command line tools (`poppler-utils`, recommended by the packages) for the manuals;
  without them the menu has no manual entry.
* For the sliders: `wpctl` (PipeWire) or `pactl` (PulseAudio); `busctl` (systemd) or
  `brightnessctl` for a built-in panel's backlight (unless sysfs is writable); `ddcutil` for an
  external monitor's brightness (optional; it needs access to `/dev/i2c-*`, usually the `i2c`
  group).
* The menu's look comes from EmulationStation (retrobox-emulationstation exports its resolved
  menu theme when it starts, when its themes are reloaded or its settings saved, and before
  each game, from patch 0008 on), so the overlay is themed in EmulationStation too, not only in
  games. The last theme is kept in `~/.local/state/retrobox/overlay-menu-theme.json` for when
  there is none yet (each boot empties the runtime directory; an older EmulationStation only
  exports it when a game starts).

## Settings

`overlay.ini` in `/etc/retrobox` and `~/.config/retrobox` (the user's file wins, key by key).
See [`data/overlay.ini`](data/overlay.ini) for every key: the menu's side, the dim opacity, the
animation length, how long the hotkey is held, sleep, the battery's percentage, the rows' height
for touch screens, the clock (on/off,
big or small, position, colour, `strftime` formats) and who scales the manual. Day and month
names follow the language, which is EmulationStation's when one is chosen there, else the
system's (`LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`): English, Spanish,
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

Run it by hand with `RUST_LOG=debug retrobox-overlay --demo` from a Wayland or X11 session, and
the daemon with `RUST_LOG=debug retrobox-overlay --daemon` (its log, when not run from a
terminal: `~/.local/state/retrobox/logs/overlay-daemon.log`). The daemon needs read access to
`/dev/input/event*` (the `input` group), like gamepadly.

## License

GPL-2.0-only, like Retrobox.

The menu icons are [Lucide](https://lucide.dev)'s, under the ISC license
([`assets/icons/LICENSE`](assets/icons/LICENSE)).
