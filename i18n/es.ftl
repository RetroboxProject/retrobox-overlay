# retrobox-overlay: es
menu-continue = Continuar
menu-save-state = Guardar estado
menu-load-state = Cargar estado
menu-manual = Manual de instrucciones
menu-disc-tray = Abrir/cerrar bandeja
menu-open-tray = Abrir bandeja
menu-close-tray = Cerrar bandeja
menu-next-disc = Siguiente disco
menu-prev-disc = Disco anterior
menu-reset = Reiniciar
menu-emulator-menu = Menú del emulador
menu-screenshot = Captura de pantalla
menu-quit = Salir del juego
# the power button and its page (sleep in games; restart and turn off
# outside them), and the sliders
menu-power = Energía
menu-standby = Modo de espera
menu-reboot = Reiniciar
menu-poweroff = Apagar
menu-brightness = Brillo
menu-volume = Volumen
# save state slots; the date is a strftime format (chrono)
slot-label = Ranura { $number }
slot-empty = Vacía
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
disc-tray-open = bandeja abierta

# help prompts (EmulationStation's words)
help-choose = ELEGIR
help-select = SELECCIONAR
help-back = VOLVER
help-close = CERRAR
help-adjust = AJUSTAR
help-mute = SILENCIAR
help-zoom = ZOOM
help-zoom-in = ACERCAR
help-zoom-out = ALEJAR
help-move = MOVER
help-page = PÁGINA

menu-audio = Audio
audio-output = Dispositivo de salida
audio-profile = Perfil
audio-no-outputs = No se encontraron salidas de audio
audio-loading = Cargando…
help-audio = AUDIO
# the on-screen keyboard's keys
osk-accents = Acentos

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DESPLAZAR
help-cancel = CANCELAR
task-yes = Sí
task-no = No
task-ok = Aceptar
task-ask = { $label } no está instalado. ¿Instalarlo ahora?
task-installing = Instalando { $label }…
task-failed = No se pudo instalar { $label }
task-done = { $label } está instalado
task-reinstall-ask = ¿Instalar { $label } de nuevo? Se actualizará.
task-reinstalling = Instalando { $label } de nuevo…
task-reinstall-failed = No se pudo actualizar { $label }
task-reinstalled = { $label } está actualizado
task-downloading = Descargando { $label }…
task-download-failed = No se pudo descargar { $label }
task-exit-status = El instalador terminó con el estado { $status }.
task-still-missing = El instalador terminó, pero { $path } no está.
task-log = Registro: { $path }
