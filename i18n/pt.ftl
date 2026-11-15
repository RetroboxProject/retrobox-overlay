# retrobox-overlay: pt
menu-continue = Continuar
menu-save-state = Guardar estado
menu-load-state = Carregar estado
menu-manual = Manual de instruções
menu-disc-tray = Abrir/fechar tabuleiro
menu-open-tray = Abrir tabuleiro
menu-close-tray = Fechar tabuleiro
menu-next-disc = Disco seguinte
menu-prev-disc = Disco anterior
menu-reset = Reiniciar
menu-emulator-menu = Menu do emulador
menu-screenshot = Captura de ecrã
menu-quit = Sair do jogo
# the system menu (sliders and sleep, in every context; sleep outside games)
menu-standby = Modo de suspensão
menu-brightness = Brilho
menu-volume = Volume
# save state slots; the date is a strftime format (chrono)
slot-label = Espaço { $number }
slot-empty = Vazio
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
disc-tray-open = tabuleiro aberto

# help prompts (EmulationStation's words)
help-choose = ESCOLHA
help-select = SELECIONAR
help-back = VOLTAR
help-close = FECHAR
help-adjust = AJUSTAR
help-mute = SILENCIAR
help-zoom = AMPLIAR
help-zoom-in = AMPLIAR ZOOM
help-zoom-out = REDUZIR ZOOM
help-move = MOVER
help-page = PÁGINA

menu-audio = Áudio
audio-output = Dispositivo de saída
audio-profile = Perfil
audio-no-outputs = Nenhuma saída de áudio encontrada
audio-loading = A carregar…
help-audio = ÁUDIO
# the on-screen keyboard's keys
osk-accents = Acentos

# the task screen: installing and downloading what Retrobox needs ({ $label } is
# the emulator or content's name)
help-scroll = DESLOCAR
help-cancel = CANCELAR
task-yes = Sim
task-no = Não
task-ok = OK
task-ask = { $label } não está instalado. Instalar agora?
task-installing = A instalar { $label }…
task-failed = Não foi possível instalar { $label }
task-done = { $label } está instalado
task-reinstall-ask = Instalar { $label } novamente? Será atualizado.
task-reinstalling = A instalar { $label } novamente…
task-reinstall-failed = Não foi possível atualizar { $label }
task-reinstalled = { $label } está atualizado
task-downloading = A transferir { $label }…
task-download-failed = Não foi possível transferir { $label }
task-exit-status = O instalador terminou com o estado { $status }.
task-still-missing = O instalador terminou, mas { $path } não existe.
task-log = Registo: { $path }
