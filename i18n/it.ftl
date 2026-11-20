# retrobox-overlay: it
menu-continue = Continua
menu-save-state = Salva stato
menu-load-state = Carica stato
menu-manual = Manuale di istruzioni
menu-info = Scheda del gioco
menu-controls = Controlli
menu-achievements = Obiettivi
menu-disc-tray = Apri/chiudi vassoio
menu-open-tray = Apri vassoio
menu-close-tray = Chiudi vassoio
menu-next-disc = Disco successivo
menu-prev-disc = Disco precedente
menu-reset = Riavvia
menu-emulator-menu = Menu dell'emulatore
menu-screenshot = Screenshot
menu-quit = Esci dal gioco
menu-save-quit = Salva ed esci
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Alimentazione
menu-standby = Modalità riposo
menu-reboot = Riavvia
menu-poweroff = Spegni
menu-performance = Profilo prestazioni
menu-brightness = Luminosità
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Slot { $number }
slot-empty = Vuoto
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A %-d %B %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disco { $current } di { $count }
disc-tray-open = vassoio aperto

# help prompts (EmulationStation's words)
help-choose = SCEGLIERE
help-select = SELEZIONA
help-hold-unsaved = TIENI PREMUTO: ESCI SENZA SALVARE
help-back = INDIETRO
help-close = CHIUDI
help-adjust = REGOLA
help-mute = MUTO
help-zoom = ZOOM
help-zoom-in = INGRANDISCI
help-zoom-out = RIDUCI
help-move = SPOSTA
help-page = PAGINA

menu-audio = Audio
audio-output = Dispositivo di uscita
audio-profile = Profilo
audio-no-outputs = Nessuna uscita audio trovata
audio-loading = Caricamento…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Accenti

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = SCORRI
help-cancel = ANNULLA
task-yes = Sì
task-no = No
task-ok = OK
task-ask = { $label } non è installato. Installarlo ora?
task-installing = Installazione di { $label }…
task-failed = Impossibile installare { $label }
task-done = { $label } è installato
task-reinstall-ask = Installare di nuovo { $label }? Verrà aggiornato.
task-reinstalling = Reinstallazione di { $label }…
task-reinstall-failed = Impossibile aggiornare { $label }
task-reinstalled = { $label } è aggiornato
task-downloading = Download di { $label }…
task-download-failed = Impossibile scaricare { $label }
task-exit-status = L'installer è terminato con lo stato { $status }.
task-still-missing = L'installer ha terminato, ma { $path } non c'è.
task-log = Registro: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Sviluppatore
info-publisher = Editore
info-released = Uscita
info-genre = Genere
info-players = Giocatori
info-rating = Valutazione
info-play-count = Volte giocato
info-play-time = Tempo di gioco
break-title = Una pausa?
break-text = Stai giocando da { $time }. Riposa gli occhi e sgranchisciti un po'.
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Puntatore del mouse
controls-wheel = Rotellina del mouse
controls-click-left = Clic sinistro
controls-click-right = Clic destro
controls-click-middle = Clic centrale
controls-command = Comando
controls-hold = tieni premuto

# the achievements page (RetroAchievements)
achievements-loading = Caricamento degli obiettivi…
achievements-failed = Impossibile raggiungere RetroAchievements
achievements-none = Questo gioco non ha obiettivi
achievements-summary = { $earned } su { $total } · { $points } su { $max } punti
