# retrobox-overlay: en
menu-continue = Continue
menu-save-state = Save state
menu-load-state = Load state
menu-manual = Instruction manual
menu-info = Game info
menu-controls = Controls
menu-achievements = Achievements
menu-disc-tray = Open/close disc tray
menu-open-tray = Open disc tray
menu-close-tray = Close disc tray
menu-next-disc = Next disc
menu-prev-disc = Previous disc
menu-reset = Reset
menu-emulator-menu = Emulator menu
menu-screenshot = Screenshot
menu-quit = Quit game
menu-save-quit = Save and quit
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Power
menu-standby = Sleep mode
menu-reboot = Restart
menu-poweroff = Turn off
menu-performance = Performance profile
menu-brightness = Brightness
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Slot { $number }
slot-empty = Empty
slot-date-format = %b %-d, %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %B %-d, %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disc { $current } of { $count }
disc-tray-open = tray open

# help prompts (EmulationStation's words)
help-choose = CHOOSE
help-select = SELECT
help-hold-unsaved = HOLD: QUIT WITHOUT SAVING
help-back = BACK
help-close = CLOSE
help-adjust = ADJUST
help-mute = MUTE
help-zoom = ZOOM
help-zoom-in = ZOOM IN
help-zoom-out = ZOOM OUT
help-move = MOVE
help-page = PAGE

menu-audio = Audio
audio-output = Output device
audio-profile = Profile
audio-no-outputs = No audio outputs found
audio-loading = Loading…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Accents

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = SCROLL
help-cancel = CANCEL
task-yes = Yes
task-no = No
task-ok = OK
task-ask = { $label } is not installed. Install it now?
task-installing = Installing { $label }…
task-failed = Could not install { $label }
task-done = { $label } is installed
task-reinstall-ask = Install { $label } again? It will be updated.
task-reinstalling = Installing { $label } again…
task-reinstall-failed = Could not update { $label }
task-reinstalled = { $label } is up to date
task-downloading = Downloading { $label }…
task-download-failed = Could not download { $label }
task-exit-status = The installer ended with status { $status }.
task-still-missing = The installer finished, but { $path } is not there.
task-log = Log: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Developer
info-publisher = Publisher
info-released = Released
info-genre = Genre
info-players = Players
info-rating = Rating
info-play-count = Times played
info-play-time = Time played
break-title = Time for a break?
break-text = You've been playing for { $time }. Rest your eyes and stretch a little.
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Mouse pointer
controls-wheel = Scroll wheel
controls-click-left = Left click
controls-click-right = Right click
controls-click-middle = Middle click
controls-command = Command
controls-hold = hold

# the achievements page (RetroAchievements)
achievements-loading = Loading the achievements…
achievements-failed = Cannot reach RetroAchievements
achievements-none = This game has no achievements
achievements-summary = { $earned } of { $total } · { $points } of { $max } points
