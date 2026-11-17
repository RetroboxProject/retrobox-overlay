# retrobox-overlay: nl
menu-continue = Doorgaan
menu-save-state = Status opslaan
menu-load-state = Status laden
menu-manual = Handleiding
menu-info = Spelinfo
menu-controls = Besturing
menu-achievements = Prestaties
menu-disc-tray = Lade openen/sluiten
menu-open-tray = Lade openen
menu-close-tray = Lade sluiten
menu-next-disc = Volgende schijf
menu-prev-disc = Vorige schijf
menu-reset = Herstarten
menu-emulator-menu = Emulatormenu
menu-screenshot = Schermafbeelding
menu-quit = Spel afsluiten
menu-save-quit = Opslaan en afsluiten
menu-quit-unsaved = Afsluiten zonder opslaan
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Aan/uit
menu-standby = Slaapstand
menu-reboot = Opnieuw opstarten
menu-poweroff = Uitschakelen
menu-brightness = Helderheid
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Slot { $number }
slot-empty = Leeg
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A %-d %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Schijf { $current } van { $count }
disc-tray-open = lade open

# help prompts (EmulationStation's words)
help-choose = KIES
help-select = SELECTEER
help-back = TERUG
help-close = SLUITEN
help-adjust = AANPASSEN
help-mute = DEMPEN
help-zoom = ZOOM
help-zoom-in = ZOOM IN
help-zoom-out = ZOOM UIT
help-move = VERPLAATS
help-page = PAGINA

menu-audio = Audio
audio-output = Uitvoerapparaat
audio-profile = Profiel
audio-no-outputs = Geen audio-uitvoer gevonden
audio-loading = Laden…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Accenten

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = SCROLLEN
help-cancel = ANNULEREN
task-yes = Ja
task-no = Nee
task-ok = OK
task-ask = { $label } is niet geïnstalleerd. Nu installeren?
task-installing = { $label } wordt geïnstalleerd…
task-failed = { $label } kon niet worden geïnstalleerd
task-done = { $label } is geïnstalleerd
task-reinstall-ask = { $label } opnieuw installeren? Het wordt bijgewerkt.
task-reinstalling = { $label } wordt opnieuw geïnstalleerd…
task-reinstall-failed = { $label } kon niet worden bijgewerkt
task-reinstalled = { $label } is up-to-date
task-downloading = { $label } wordt gedownload…
task-download-failed = { $label } kon niet worden gedownload
task-exit-status = Het installatieprogramma eindigde met status { $status }.
task-still-missing = Het installatieprogramma is klaar, maar { $path } ontbreekt.
task-log = Logboek: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Ontwikkelaar
info-publisher = Uitgever
info-released = Uitgebracht
info-genre = Genre
info-players = Spelers
info-rating = Waardering
info-play-count = Keer gespeeld
info-play-time = Speeltijd
play-time = { $session } · { $total } in totaal
duration-minutes = { $minutes } min
duration-hours = { $hours } u
duration-hours-minutes = { $hours } u { $minutes } min
controls-pointer = Muisaanwijzer
controls-wheel = Scrollwiel
controls-click-left = Linkerklik
controls-click-right = Rechterklik
controls-click-middle = Middelklik
controls-command = Opdracht
controls-hold = ingedrukt houden

# the achievements page (RetroAchievements)
achievements-loading = Prestaties laden…
achievements-failed = RetroAchievements is niet bereikbaar
achievements-none = Dit spel heeft geen prestaties
achievements-summary = { $earned } van { $total } · { $points } van { $max } punten
