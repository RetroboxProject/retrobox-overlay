# retrobox-overlay

The in-game menu of [Retrobox](https://github.com/RetroboxProject/retrobox-core). Hold the
controller's Home/Guide button for a moment, press Hotkey+B (Nintendo's B, the south button)
or press Ctrl+Shift+O (or Shift+F7), while playing: a
side menu slides in over the dimmed game, drawn with the menu style of the active
EmulationStation theme, with a clock and the date in the top corner of the other side.

It is a small Rust program (a few MB, ~35 ms from start to the menu on screen). What stays
resident is a small daemon that only reads the controllers and keyboards; the menu itself runs
only while it is shown.

## How it fits in Retrobox

* **Opening and closing.** Retrobox starts `retrobox-inputd`, the input daemon (a binary of
  its own, with no screen), with EmulationStation; `retrobox-overlay --daemon`, what older
  launchers run, starts it too. It reads every controller and keyboard (evdev, without grabbing them: EmulationStation and
  the games get every input as before) and opens the overlay (`retrobox-overlay --show`, a
  process of its own) when a controller's dedicated hotkey (Home/Guide) is held alone for
  `hold_ms` (750 ms), or on Ctrl+Shift+O (Shift+F7 too, for keyboards whose function keys need
  Fn). A controller without a Home/Guide button has Select as
  its hotkey, as in EmulationStation, and Select belongs to the game: holding it opens nothing
  there. Hotkey+B, pressing B while the hotkey is held, opens it at once on every controller,
  Home/Guide or Select alike. Any other button, the d-pad or a stick
  pushed meanwhile cancels the hold until everything is released, so the hotkey's other
  combinations never open it. A hold is used rather than a press because the hotkey is also
  the modifier of every Hotkey+… combination: a press alone could not be told apart from the
  start of one. `retrobox-overlay` started on its own asks the daemon to open or close the
  overlay (`SIGUSR1`); without a daemon it shows the overlay itself, as before. Only one
  overlay runs at a time (`overlay.lock` in Retrobox's runtime directory); a second start
  closes it. The `overlay` setting of retrobox-core (global, per system or per game) turns it
  off over a game.
* **Pad-to-keyboard mappings** (what gamepadly did). When `session.json` names a profile
  (`pad_profile`, which the launcher composes from the game's own and the standard hotkeys), the
  daemon maps the game's controllers with it while the game runs: evmapy's `.keys` format with
  gamepadly's additions (`condition`, `cooldown`, `tap`, `mouse` and `scroll` sticks), read the
  way gamepadly reads it. Each controller gets its player's mappings (`actions_player<N>`, else
  `mappings`), bound to its buttons by `es_input.cfg`; every keyboard gets `actions_keyboard`.
  Keys and mouse buttons go to a virtual keyboard and mouse (`/dev/uinput`, the `input` group,
  as for gamepadly), `exec` commands run in the background. Nothing is grabbed; while the overlay
  is open (it grabs the controllers) what the mappings held is let go.
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
* **Layout.** The game's title (bold: the title font's bold face when it sits next to it, as
  `Name-Bold.ttf`, else drawn a little thicker) heads the menu; under it, the entries, which
  scroll when they do not all fit; pinned at the bottom (they never scroll, and hide only while
  a save state page is open), three round buttons, *continue* (back to the game), *screenshot*
  (when the emulator takes them) and *quit*, and under them the sliders. Up and Down go through
  the entries, the buttons' row and the sliders, round (down from the last entry or up from the
  first one reaches them); Left/Right choose a button; R1 (Page Down too) jumps from the entries
  to the buttons and back, L1 (Page Up) back to the entries. A button highlighted for over a
  second (selected or under the pointer) tells what it does in a small bubble above it, gone as
  soon as something else is highlighted. The selected button gets a ring in the text's colour
  around the selector's disc, so it stands out whatever the theme.
* **The emulator.** The overlay only asks; the launcher does. While the game runs, the launcher
  serves a control channel (a Unix socket, `control` in `session.json`) and carries out each
  request the best way the emulator allows: RetroArch's command pipe (its standard input,
  `stdin_cmd_enable`), else a key chord on a virtual keyboard the launcher creates when the game
  starts, else a command line. The menu offers what the channel says the emulator can do:
  load and save state, the disc tray and next/previous disc, reset, the emulator's menu
  (RetroArch's only, for now), each with its icon, and a screenshot (a round button).
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
  menu needs; the mouse and touch work too. On X11 it always takes the focus: window managers
  keep a focused fullscreen window above everything (xfwm4 even shows it outside its
  compositor), so the overlay would open behind the game. Opened from the keyboard (Ctrl+Shift+O)
  it takes the keyboard, on demand and not exclusively: when the compositor gives it to another window
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
* **On-screen keyboard** (`retrobox-overlay --osk`, first version). Hotkey+Y (Nintendo's Y) in an
  emulator or app whose feature file in retrobox-core lists `osk` (the web browsers): a keyboard
  slides up from the bottom of the screen, in the menu's theme colours and font, over the game
  without dimming it. The d-pad or stick moves over the keys, A types, Y is backspace, X space,
  Start enter, L and R move the cursor, R2 toggles shift (locked on at the start), L2 the symbols
  layer, B the accents page, Home closes it, and a click or a tap outside the keyboard too. The
  keys around the letters: Shift (2x2), Tab and symbols on the left, the overlay button
  (opens the standard overlay in its place), backspace and Enter (2x2) on the right; and the
  bottom row has Close, the cursors on either side of the space bar, and Accents. The buttons
  are those `es_input.cfg` gives each controller (its `b`, `a`, `x`, `y`, `start`... numbered
  as SDL's evdev driver does, matched by GUID or name), as the emulators do; without an entry
  for the pad, the evdev names (and the Xbox ones' quirk) decide. Icons are Lucide's (ISC), and
  the buttons' the theme's help icons, else EmulationStation's `button_<name>.svg`, else text.
  Home closes it, and a click or a tap outside the keyboard too. The right stick sets its size
  (up bigger, down smaller: 10 % to 75 % of the screen's width, 75 % by default) and R3 moves
  it between the bottom and the top of the screen; both are saved in the user's `overlay.ini`
  (`[osk]` `width`, `position`) when the keyboard closes, and only if they changed. It never takes the keyboard
  focus: the game keeps it, and the keys are typed on the launcher's virtual keyboard through the
  control channel (`key down|up`), so the overlay and the keyboard are never open together (they
  share `overlay.lock`: starting either while the other is open closes it). The keys are physical
  keys, labelled and typed from the active xkb keymap (the compositor's on Wayland; else the
  `[osk]` names in `overlay.ini`, `XKB_DEFAULT_*` or `/etc/default/keyboard`; else US): the
  symbols page holds what the layout types with shift and AltGr, and the accents page (A) the
  accented letters it can type, directly or as a dead key and a letter. Held backspace and cursor
  keys repeat; with a physical keyboard the arrow keys move, Enter types and Escape closes (the
  keyboards are grabbed while it is open, so nothing else reaches the game); the keyboard slides
  out when it closes. Keys are typed on a thread of their own, and the controller buttons on the
  keys are the theme's help icons, written where it has none.
* **Leaving the game** runs `retrobox-emukill`.
* **Outside games** (in EmulationStation) the same hold opens the system's menu, like the
  Switch's quick settings: *Sleep mode* (`systemctl suspend`; `standby` in `[system]`), and the
  brightness and volume sliders, which games get too, pinned under the entries.
* **Sliders.** Left/Right (d-pad, stick, arrow keys) move them by 5 %, holding repeats; the
  mouse or a finger moves them with a tap or a drag on the track. They have no label, only
  their icon (a sun, a speaker that shows the level), so the track takes the row's width; their
  rows are 20 % taller than the other entries, whatever `row_scale` says, with a thicker track
  and a larger knob, for a finger. Changes apply at once, from a thread
  that only applies the latest value (a held direction asks for many), and the last one is
  applied before the overlay exits. The sliders are there when the menu shows: the volume is
  read in a thread from the moment the overlay starts (the menu waits for it 60 ms at most),
  and finding the brightness's screen starts no program at all. A slider is left out when
  there is nothing to control.
  * **Brightness** of a built-in screen (eDP, LVDS, DSI: a laptop's, a Nintendo Switch's),
    and only when it is the primary one: the only screen that is on (`/sys/class/drm`), else the
    primary screen of the display settings: X11's primary output (RandR), KDE's on Wayland
    (`kscreen-doctor`, priority 1), else the output the overlay shows on (other Wayland
    compositors put it on the screen in use). External monitors get no slider. The kernel's
    backlight is written in sysfs (retrobox-core's udev rule lets the `video` group write it:
    no program per change), else through logind (`busctl`, allowed to the active session
    without root), else `brightnessctl`; the first way that works is kept, a refused change
    (NVIDIA's `nvidia_0` refuses one while still fading to the last one) is tried again rather
    than another way, and it never goes below 5 % so the screen does not go black. The
    brightness does not follow the slider while it moves: once it has rested for 250 ms, it
    glides from where it is straight to the slider's value (down when that is lower, up when
    higher, never both), easing in and out over 0.4 s at most (less for a short change), a
    write every 35 ms and never the same value twice; moving the slider again stops it there
    until the slider rests again.
  * **Volume** of the default output: PipeWire (`wpctl`), else PulseAudio (`pactl`, which also
    works with PipeWire's PulseAudio server). At 0 the output is muted; above, unmuted. A
    (Enter, a click or a tap off its track) on the volume's slider mutes or unmutes it, its
    level kept: the speaker crossed out and the slider faded; moving it unmutes.
    The slider follows changes made elsewhere (volume keys, another mixer, a new default
    output with its own volume) at once: the sound server is watched for events (`pw-dump
    --monitor` or `pactl subscribe`), never polled. When EmulationStation's option to set the
    volume with the right stick (`RightStickVolume` in `es_settings.cfg`, on by default) is on, the right stick moves this slider wherever the selection is; not in the pages (slots,
    audio devices) nor the manual: a page that opens with the stick held takes no more of it.
    The help does not mention it.
  * **Audio page.** X on a controller (while the volume's slider is selected) or F2 (anywhere)
    opens a page of choices: the system's default output device and, under it, the profile of
    that device's card (stereo, headset, HDMI...), with a check mark on the current ones. The change
    is system-wide, as in pavucontrol or KDE's sound settings, and the page follows what changes
    outside while it is open. PipeWire is used when it is there (`wpctl` and `pw-dump`), else
    PulseAudio (`pactl` with JSON output, PulseAudio 16 or later). The "off" profile is not
    offered, so a card cannot be switched off from the overlay; profiles that cannot be used
    now (nothing plugged in) are shown dimmed. There is no touch entry to the page.
* **Touch screens and dragging.** A finger is the left button: a tap opens an entry (or sets a
  slider where it lands on its track), a drag up or down scrolls the entries smoothly (pixel by
  pixel), a drag along a slider moves it; the mouse drags the same way. Entries open on
  release, so a drag never opens one. `row_scale` in `[touch]` makes the rows taller than
  EmulationStation's (the text stays the same size, the entries' and sliders' icons grow with
  them) so they are easier to hit: 1.5 to 1.66 suits small touch screens.
* **Status line**, above the menu's header, half as large again as the small text, its icons
  centred on the digits: the network (a cable wins over Wi-Fi, whose icon
  shows the signal), the battery (the computer's own: a controller's is left out) with its
  percentage (`battery_percent` in `[status]`), and the time when the clock is the small one
  (`size = small` in `[clock]`, with its own format, `small_format`); the big clock keeps its
  corner. They are read again every second, from sysfs.
* **Installing and downloading** (`retrobox-overlay --task`). The screen Retrobox shows when it
  needs something it has not got: the emulator a game needs (it asks whether to install it
  first), or the default theme, bezels and shaders before EmulationStation starts (it runs the
  download at once and closes when it works). It is a process of its own, not the daemon's, so
  it works before EmulationStation is up and leaves nothing resident. A rounded window in
  the menu's colours (EmulationStation's defaults without its theme) over Retrobox's background
  picture holds the question (in capitals, like EmulationStation's dialogs), the buttons
  (controller, keyboard, mouse or touch; the chosen one, or the one under the pointer, filled,
  the others with a border, as in EmulationStation) and the installer's output, with the help prompts below it. The question alone
  makes a small window, which grows once the installer runs. The output is white on
  black, in a fixed-width font, at least 80 columns wide (the font shrinks to fit them), and runs
  in a pseudo-terminal: what asks for something, such as sudo's password,
  gets the keyboard's keys, as in any terminal. Page Up/Down and the wheel scroll the output;
  a selection made with the pointer is copied when it ends (Ctrl+Shift+C copies it, or all
  the output; Ctrl+Shift+A selects it all), with `wl-copy`, `xclip` or `xsel`. The full output
  is saved in the log file it is given.

      retrobox-overlay --task --label NAME --log FILE [--check FILE] [--background PNG]
                       [--pad DEVICE]... [--lang CODE] [--download | --reinstall]
                       -- COMMAND [ARGUMENT]...

  `--check` is a file the command must have left (a command that works but leaves nothing is
  a failure), `--reinstall` offers to install again what is installed (to update it).
  Exit status: 0 done, 10 declined or cancelled (a download is cancelled with Back), 11 failed
  (the output stays on screen until the user closes it), 12 no screen could be opened.
  `retrobox-overlay --held` exits with 0 while a controller has L1 and R1 held down, or a
  keyboard Ctrl and Shift (read from the kernel, nothing grabbed; keyboards need the `input`
  group): holding them while a game loads is how the launcher is asked to offer the
  emulator's installation again.

## Requirements

* A Wayland compositor with `wlr-layer-shell` (KWin, labwc, sway and other wlroots compositors),
  or an X11 server: Plasma on X11, for instance where the GPU driver has no good Wayland support
  (Nvidia Tegra, as on Switchroot). Without layer-shell (GNOME) it falls back to X11 through
  XWayland.
* On Wayland, `wp_alpha_modifier` (KWin 6.1+, labwc 0.9+) adds the fade-in; without it the menu
  only slides. On X11 a compositing manager (KWin's, picom...) makes the dim translucent and
  fades the menu in (`_NET_WM_WINDOW_OPACITY`); without one (xfwm4 with its compositor off,
  for instance) a window cannot be translucent, so the dim is a darkened copy of what the
  screen showed when the overlay opened (the game goes on under it, unseen until it closes).
* retrobox-core and retrobox-emulationstation (the packages depend on both).
* poppler's command line tools (`poppler-utils`, recommended by the packages) for the manuals;
  without them the menu has no manual entry.
* For the sliders: `wpctl` (PipeWire) or `pactl` (PulseAudio); for the audio page and for
  following the volume: `pw-dump` (PipeWire's tools) or `pactl`; `busctl` (systemd) or
  `brightnessctl` for a built-in panel's backlight (unless sysfs is writable);
  `kscreen-doctor` to know KDE's primary screen on Wayland with more than one screen on.
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

A Cargo workspace: the overlay (this package, `src/`), the input daemon
(`crates/retrobox-inputd`) and what both share (`crates/retrobox-input`: the device probe,
`es_input.cfg` and `es_settings.cfg`, the paths, the locks, the logs). Every command below
covers the three.

```bash
cargo build --release          # target/release/retrobox-overlay and retrobox-inputd
cargo test && cargo clippy     # what CI checks (clippy pedantic, warnings are errors)
cargo deb --no-build           # .deb  (cargo install cargo-deb)
cargo generate-rpm             # .rpm  (cargo install cargo-generate-rpm)
```

It needs `libxkbcommon` (development files to build). GitHub Actions build the `.deb` and
`.rpm` for amd64 and arm64 on every `v*` tag and publish them with a `SHA256SUMS` file, like the
other Retrobox components.

Run it by hand with `RUST_LOG=debug retrobox-overlay --demo` from a Wayland or X11 session, and
the daemon with `RUST_LOG=debug retrobox-inputd` (its log, when not run from a terminal:
`~/.local/state/retrobox/logs/inputd.log`). The daemon needs read access to
`/dev/input/event*` (the `input` group), like gamepadly.

## License

GPL-2.0-only, like Retrobox.

The menu icons are [Lucide](https://lucide.dev)'s, under the ISC license
([`assets/icons/LICENSE`](assets/icons/LICENSE)).
