# retrobox-overlay: ca
menu-continue = Continua
menu-save-state = Desa l'estat
menu-load-state = Carrega l'estat
menu-manual = Manual d'instruccions
menu-info = Fitxa del joc
menu-controls = Controls
menu-disc-tray = Obre/tanca la safata
menu-open-tray = Obre la safata
menu-close-tray = Tanca la safata
menu-next-disc = Disc següent
menu-prev-disc = Disc anterior
menu-reset = Reinicia
menu-emulator-menu = Menú de l'emulador
menu-screenshot = Captura de pantalla
menu-quit = Surt del joc
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Energia
menu-standby = Mode de repòs
menu-reboot = Reinicia
menu-poweroff = Apaga
menu-brightness = Brillantor
menu-volume = Volum
# save state slots; the date is a strftime format (chrono)
slot-label = Ranura { $number }
slot-empty = Buida
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %-d %B de %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disc { $current } de { $count }
disc-tray-open = safata oberta

# help prompts (EmulationStation's words)
help-choose = TRIAR
help-select = SELECCIONAR
help-back = ENRERE
help-close = TANCAR
help-adjust = AJUSTAR
help-mute = SILENCIAR
help-zoom = ZOOM
help-zoom-in = APROPAR
help-zoom-out = ALLUNYAR
help-move = MOURE
help-page = PÀGINA

menu-audio = Àudio
audio-output = Dispositiu de sortida
audio-profile = Perfil
audio-no-outputs = No s'han trobat sortides d'àudio
audio-loading = Carregant…
help-audio = ÀUDIO
# the on-screen keyboard's keys
osk-accents = Accents

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DESPLAÇA
help-cancel = CANCEL·LA
task-yes = Sí
task-no = No
task-ok = D'acord
task-ask = { $label } no està instal·lat. Voleu instal·lar-lo ara?
task-installing = S'està instal·lant { $label }…
task-failed = No s'ha pogut instal·lar { $label }
task-done = { $label } està instal·lat
task-reinstall-ask = Voleu instal·lar { $label } de nou? S'actualitzarà.
task-reinstalling = S'està instal·lant { $label } de nou…
task-reinstall-failed = No s'ha pogut actualitzar { $label }
task-reinstalled = { $label } està actualitzat
task-downloading = S'està baixant { $label }…
task-download-failed = No s'ha pogut baixar { $label }
task-exit-status = L'instal·lador ha acabat amb l'estat { $status }.
task-still-missing = L'instal·lador ha acabat, però { $path } no hi és.
task-log = Registre: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Desenvolupador
info-publisher = Editor
info-released = Llançament
info-genre = Gènere
info-players = Jugadors
info-rating = Valoració
info-play-count = Vegades jugat
info-play-time = Temps jugat
play-time = { $session } · { $total } en total
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Punter del ratolí
controls-wheel = Roda del ratolí
controls-click-left = Clic esquerre
controls-click-right = Clic dret
controls-click-middle = Clic central
controls-command = Ordre
controls-hold = mantenir
