# retrobox-overlay: gl
menu-continue = Continuar
menu-save-state = Gardar estado
menu-load-state = Cargar estado
menu-manual = Manual de instrucións
menu-info = Ficha do xogo
menu-controls = Controis
menu-achievements = Logros
menu-disc-tray = Abrir/pechar a bandexa
menu-open-tray = Abrir a bandexa
menu-close-tray = Pechar a bandexa
menu-next-disc = Seguinte disco
menu-prev-disc = Disco anterior
menu-reset = Reiniciar
menu-emulator-menu = Menú do emulador
menu-screenshot = Captura de pantalla
menu-quit = Saír do xogo
menu-save-quit = Gardar e saír
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Enerxía
menu-standby = Modo de repouso
menu-reboot = Reiniciar
menu-poweroff = Apagar
menu-performance = Perfil de rendemento
menu-brightness = Brillo
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Rañura { $number }
slot-empty = Baleira
slot-date-format = %-d %b %Y · %H:%M
# strftime formats of the clock (chrono), used unless overlay.ini sets its own
clock-date-format = %A, %-d de %B de %Y
clock-time-format = %H:%M:%S
clock-time-format-12h = %I:%M:%S %p
# the small clock, in the status line (clock size = small)
clock-small-format = %H:%M
clock-small-format-12h = %-I:%M %p
# the disc line under the game's name (games of several discs)
disc-status = Disco { $current } de { $count }
disc-tray-open = bandexa aberta

# help prompts (EmulationStation's words)
help-choose = ESCOLLER
help-select = SELECCIONAR
help-hold-unsaved = MANTER: SAÍR SEN GARDAR
help-back = VOLVER
help-close = PECHAR
help-adjust = AXUSTAR
help-mute = SILENCIAR
help-zoom = AUMENTAR
help-zoom-in = ACHEGAR
help-zoom-out = ALONXAR
help-move = MOVER
help-page = PÁXINA

menu-audio = Audio
audio-output = Dispositivo de saída
audio-profile = Perfil
audio-no-outputs = Non se atoparon saídas de audio
audio-loading = Cargando…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Acentos

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DESPRAZAR
help-cancel = CANCELAR
task-yes = Si
task-no = Non
task-ok = Aceptar
task-ask = { $label } non está instalado. Instalalo agora?
task-installing = Instalando { $label }…
task-failed = Non se puido instalar { $label }
task-done = { $label } está instalado
task-reinstall-ask = Instalar { $label } de novo? Actualizarase.
task-reinstalling = Instalando { $label } de novo…
task-reinstall-failed = Non se puido actualizar { $label }
task-reinstalled = { $label } está actualizado
task-downloading = Descargando { $label }…
task-download-failed = Non se puido descargar { $label }
task-exit-status = O instalador rematou co estado { $status }.
task-still-missing = O instalador rematou, pero { $path } non está.
task-log = Rexistro: { $path }

# the game's page, the time played (the menu's status line), the controls page
info-developer = Desenvolvedor
info-publisher = Editor
info-released = Lanzamento
info-genre = Xénero
info-players = Xogadores
info-rating = Valoración
info-play-count = Veces xogado
info-play-time = Tempo xogado
break-title = Un descanso?
break-text = Levas { $time } xogando. Descansa a vista e estírate un pouco.
duration-minutes = { $minutes } min
duration-hours = { $hours } h
duration-hours-minutes = { $hours } h { $minutes } min
controls-pointer = Punteiro do rato
controls-wheel = Roda do rato
controls-click-left = Clic esquerdo
controls-click-right = Clic dereito
controls-click-middle = Clic central
controls-command = Comando
controls-hold = manter

# the achievements page (RetroAchievements)
achievements-loading = Cargando os logros…
achievements-failed = Non se pode conectar con RetroAchievements
achievements-none = Este xogo non ten logros
achievements-summary = { $earned } de { $total } · { $points } de { $max } puntos
