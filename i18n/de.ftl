# retrobox-overlay: de
menu-continue = Fortsetzen
menu-save-state = Zustand speichern
menu-load-state = Zustand laden
menu-manual = Spielanleitung
menu-info = Spielinfo
menu-controls = Steuerung
menu-achievements = Erfolge
menu-disc-tray = Laufwerk öffnen/schließen
menu-open-tray = Laufwerk öffnen
menu-close-tray = Laufwerk schließen
menu-next-disc = Nächste Disc
menu-prev-disc = Vorherige Disc
menu-reset = Neustart
menu-emulator-menu = Emulator-Menü
menu-screenshot = Bildschirmfoto
menu-quit = Spiel beenden
menu-save-quit = Speichern und beenden
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Energie
menu-standby = Standby-Modus
menu-reboot = Neu starten
menu-poweroff = Ausschalten
menu-brightness = Helligkeit
menu-volume = Lautstärke
# save state slots; the date is a strftime format (chrono)
slot-label = Platz { $number }
slot-empty = Leer
slot-date-format = %-d. %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %-d. %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disc { $current } von { $count }
disc-tray-open = Laufwerk offen

# help prompts (EmulationStation's words)
help-choose = AUSWÄHLEN
help-select = AUSWÄHLEN
help-hold-unsaved = HALTEN: BEENDEN OHNE SPEICHERN
help-back = ZURÜCK
help-close = SCHLIESSEN
help-adjust = ANPASSEN
help-mute = STUMM
help-zoom = ZOOM
help-zoom-in = HINEINZOOMEN
help-zoom-out = HERAUSZOOMEN
help-move = BEWEGEN
help-page = SEITE

menu-audio = Audio
audio-output = Ausgabegerät
audio-profile = Profil
audio-no-outputs = Keine Audioausgänge gefunden
audio-loading = Wird geladen…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Akzente

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = SCROLLEN
help-cancel = ABBRECHEN
task-yes = Ja
task-no = Nein
task-ok = OK
task-ask = { $label } ist nicht installiert. Jetzt installieren?
task-installing = { $label } wird installiert…
task-failed = { $label } konnte nicht installiert werden
task-done = { $label } ist installiert
task-reinstall-ask = { $label } erneut installieren? Es wird aktualisiert.
task-reinstalling = { $label } wird erneut installiert…
task-reinstall-failed = { $label } konnte nicht aktualisiert werden
task-reinstalled = { $label } ist aktuell
task-downloading = { $label } wird heruntergeladen…
task-download-failed = { $label } konnte nicht heruntergeladen werden
task-exit-status = Das Installationsprogramm endete mit Status { $status }.
task-still-missing = Das Installationsprogramm ist fertig, aber { $path } fehlt.
task-log = Protokoll: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Entwickler
info-publisher = Publisher
info-released = Erschienen
info-genre = Genre
info-players = Spieler
info-rating = Bewertung
info-play-count = Mal gespielt
info-play-time = Spielzeit
play-time = { $session } · { $total } insgesamt
break-title = Zeit für eine Pause?
break-text = Du spielst seit { $time }. Gönn deinen Augen eine Pause und streck dich ein wenig.
duration-minutes = { $minutes } Min.
duration-hours = { $hours } Std.
duration-hours-minutes = { $hours } Std. { $minutes } Min.
controls-pointer = Mauszeiger
controls-wheel = Mausrad
controls-click-left = Linksklick
controls-click-right = Rechtsklick
controls-click-middle = Mittelklick
controls-command = Befehl
controls-hold = halten

# the achievements page (RetroAchievements)
achievements-loading = Erfolge werden geladen…
achievements-failed = RetroAchievements ist nicht erreichbar
achievements-none = Dieses Spiel hat keine Erfolge
achievements-summary = { $earned } von { $total } · { $points } von { $max } Punkten
